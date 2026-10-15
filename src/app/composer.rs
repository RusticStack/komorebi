//! The composer: drafts live in Rust, keyed by id, and the composer's
//! fields in the window are a view of the active one. One draft shows at a
//! time; the others wait in memory until it is sent or discarded.
//!
//! Drafts worth keeping are also saved to the cache (`db::DraftRow`): two
//! seconds after the last edit, and at once when the composer closes, another
//! draft takes its place, its account changes or the window closes. Their
//! attachments are copied into the drafts folder once, when added.
//!
//! The sync worker also copies saved drafts to the server's Drafts folder
//! (`Job::SaveDraft`): at most every 30 seconds while one is typed, and when
//! the composer closes or another draft takes its place.

use super::*;
use crate::mail::render;
use komorebi::mail::rich as model;
use komorebi::mail::rich::edit::Editor;

mod markup;
pub(super) mod rich;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct DraftId(u64);

#[cfg(test)]
impl DraftId {
    pub(super) fn for_test(n: u64) -> Self {
        DraftId(n)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Kind {
    #[default]
    New,
    Reply,
    ReplyAll,
    Forward,
}

impl Kind {
    /// The `drafts.kind` value.
    fn code(self) -> i64 {
        match self {
            Kind::New => 0,
            Kind::Reply => 1,
            Kind::ReplyAll => 2,
            Kind::Forward => 3,
        }
    }

    fn from_code(code: i64) -> Self {
        match code {
            1 => Kind::Reply,
            2 => Kind::ReplyAll,
            3 => Kind::Forward,
            _ => Kind::New,
        }
    }

    fn title(self) -> String {
        match self {
            Kind::New => t!(invoke_title_new),
            Kind::Reply => t!(invoke_title_reply),
            Kind::ReplyAll => t!(invoke_title_reply_all),
            Kind::Forward => t!(invoke_title_forward),
        }
    }
}

/// Encoded attachment size past which many servers reject a message.
const MAX_ATTACHMENTS_BYTES: u64 = 25 * 1024 * 1024;
/// Komorebi's own limit on encoded attachments, checked before reading.
const MAX_ATTACH_HARD: u64 = 100 * 1024 * 1024;

/// A draft is saved this long after the last edit.
const AUTOSAVE_AFTER: Duration = Duration::from_secs(2);

/// While a draft is typed, its autosaves copy it to the server at most
/// this often.
const SERVER_SAVE_EVERY: Duration = Duration::from_secs(30);

/// Recipient suggestions shown at most, and the characters of an address
/// typed before they show.
const SUGGESTIONS: usize = 6;
const SUGGEST_AFTER: usize = 2;

/// The address fields, in the order of `Draft::errors`.
const ADDRESS_FIELDS: [ComposeField; 3] = [ComposeField::To, ComposeField::Cc, ComposeField::Bcc];

/// An attachment of a draft, and its saved copy once the draft is saved.
#[derive(Clone)]
pub(super) struct Attached {
    pub file: OutAttachment,
    /// Its `draft_attachments` row.
    pub saved: Option<i64>,
}

impl From<OutAttachment> for Attached {
    fn from(file: OutAttachment) -> Self {
        Attached { file, saved: None }
    }
}

#[derive(Default)]
pub(super) struct Draft {
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
}

/// The lines a reply's or forward's quote starts with, in the language
/// shown (`Heads::shown`; English in unit tests, which have no window).
pub(super) struct Heads {
    pub lang: komorebi::i18n::Lang,
    /// "On {date}, {name} wrote:" with its two places.
    wrote: fn(&str, &str) -> String,
    pub forwarded: String,
    pub from: String,
    pub date: String,
    pub subject: String,
    pub to: String,
}

impl Heads {
    pub fn shown(lang: komorebi::i18n::Lang) -> Self {
        Heads {
            lang,
            wrote: |date, name| t!(invoke_quote_wrote, date.into(), name.into()),
            forwarded: t!(invoke_forwarded),
            from: t!(invoke_forward_from),
            date: t!(invoke_forward_date),
            subject: t!(invoke_forward_subject),
            to: t!(invoke_forward_to),
        }
    }

    #[cfg(test)]
    pub fn en() -> Self {
        Heads {
            lang: komorebi::i18n::Lang::En,
            wrote: |date, name| format!("On {date}, {name} wrote:"),
            forwarded: FORWARDED.into(),
            from: "From".into(),
            date: "Date".into(),
            subject: "Subject".into(),
            to: "To".into(),
        }
    }
}

/// The body before a formatting edit and its selection then. Replacing the
/// text clears the field's own undo history, so Ctrl+Z puts this back
/// while the body is still `after` (typing since is undone first).
pub(super) struct Unformat {
    before: String,
    after: String,
    anchor: usize,
    cursor: usize,
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
                format::full_date(heads.lang, m.date),
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
    fn to_row(&self, id: i64) -> db::DraftRow {
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

    pub fn field(&self, field: ComposeField) -> &str {
        match field {
            ComposeField::To => &self.to,
            ComposeField::Cc => &self.cc,
            ComposeField::Bcc => &self.bcc,
            ComposeField::Subject => &self.subject,
            ComposeField::Body => &self.body,
        }
    }

    /// The user typed into `field`. An address error already shown is
    /// checked again at once, so it clears as soon as the address is right.
    pub fn edit(&mut self, field: ComposeField, text: &str) {
        let slot = match field {
            ComposeField::To => &mut self.to,
            ComposeField::Cc => &mut self.cc,
            ComposeField::Bcc => &mut self.bcc,
            ComposeField::Subject => &mut self.subject,
            ComposeField::Body => &mut self.body,
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
    pub fn validate(&mut self, field: ComposeField) {
        if let Some(i) = address_index(field) {
            self.errors[i] = address_error(self.field(field));
        }
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
        if encoded_size(attachments_size(&self.attachments)) > MAX_ATTACHMENTS_BYTES {
            return Some(SendBlock::AttachmentsOver);
        }
        None
    }
}

fn address_index(field: ComposeField) -> Option<usize> {
    ADDRESS_FIELDS.iter().position(|f| *f == field)
}

/// Why Send is off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum SendBlock {
    /// An address of this field (To, Cc or Bcc) is not valid.
    FixAddress(ComposeField),
    NoRecipient,
    /// The attachments are over what servers take.
    AttachmentsOver,
}

/// What the send guards ask about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Guard {
    NoSubject,
    NoAttachment,
    Both,
}

/// The address at fault in an address field as typed (none: all fine).
pub(super) fn address_error(text: &str) -> Option<String> {
    mail::smtp::first_invalid_address(text)
}

#[derive(Default)]
pub(super) struct Drafts {
    items: HashMap<DraftId, Draft>,
    active: Option<DraftId>,
    next: u64,
    /// The recipient suggestions showing for the active draft.
    suggest: Option<Suggest>,
    /// Drafts shown in tabs of their own (`app::tabs`): never dropped when
    /// another one takes the composer, even with nothing written.
    tabbed: HashSet<DraftId>,
}

/// Recipient suggestions from the address index for the address being
/// typed in an address field.
pub(super) struct Suggest {
    field: ComposeField,
    /// Where the caret is taken to be (`caret_after_edit`).
    cursor: usize,
    items: Vec<db::Recipient>,
}

impl Drafts {
    /// Add `draft` and make it the active one (see `switch_to`).
    pub fn open(&mut self, draft: Draft) -> DraftId {
        self.next += 1;
        let id = DraftId(self.next);
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
        let id = DraftId(self.next);
        self.items.insert(id, draft);
        id
    }

    /// Make `id` the active draft, dropping nothing (a tab switch).
    pub fn show(&mut self, id: DraftId) {
        if self.items.contains_key(&id) {
            self.active = Some(id);
        }
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

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// The newest draft worth keeping other than the active one.
    #[cfg(test)]
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
pub(super) fn send_guard(d: &Draft) -> Option<Guard> {
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
pub(super) fn mentions_attachment(body: &str) -> bool {
    body.lines()
        .take_while(|l| !is_forwarded_line(l) && *l != "-- ")
        .filter(|l| !l.trim_start().starts_with('>'))
        .flat_map(|l| l.split(|c: char| !c.is_alphanumeric()))
        .any(|w| {
            let w = w.to_lowercase();
            w.starts_with("attach") || w.starts_with("anex")
        })
}

/// The line a forward's quoted message starts with (English; the
/// composer writes it in the language shown, `Heads::forwarded`).
#[cfg(test)]
const FORWARDED: &str = "---------- Forwarded message ----------";

/// A forward's heading line in any language: `---------- ... ----------`.
fn is_forwarded_line(line: &str) -> bool {
    let line = line.trim_end();
    line.len() > 22 && line.starts_with("---------- ") && line.ends_with(" ----------")
}

/// Where a reply's or forward's quote starts in `body` (a byte offset at
/// a line start), for folding it: a forward's heading line, or the line
/// above a trailing run of `>` lines (the attribution). `None`: no quote.
fn quote_start(body: &str) -> Option<usize> {
    let mut at = 0;
    let mut starts = Vec::new();
    let mut found = None;
    for line in body.split_inclusive('\n') {
        if is_forwarded_line(line) {
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
fn with_quote(body: &str, quote: &str) -> String {
    if body.is_empty() || body.ends_with('\n') || quote.starts_with(['\n', '\r']) {
        format!("{body}{quote}")
    } else {
        format!("{body}\n\n{quote}")
    }
}

/// The text of a draft above its reply or forward quote (the whole text
/// with none): what the Drafts row previews.
pub(crate) fn above_quote(body: &str) -> &str {
    &body[..quote_start(body).unwrap_or(body.len())]
}

/// What saving a draft did to the cache.
#[derive(Debug, PartialEq, Eq)]
enum Stored {
    Nothing,
    Inserted,
    Updated,
    /// With the copy on the server, to remove there.
    Deleted(Option<db::ServerCopy>),
}

/// Save `d`: a draft worth keeping is written (inserted, with copies of its
/// attachments, the first time; then one UPDATE of its fields), and one that
/// is not (emptied, or never changed by the user) loses its saved row.
/// `email` is the From address its Message-ID is minted with.
fn store(db: &Db, d: &mut Draft, email: &str) -> anyhow::Result<Stored> {
    if !d.keep() {
        d.dirty = false;
        d.saved_at = None;
        let Some(row) = d.row.take() else {
            return Ok(Stored::Nothing);
        };
        for a in &mut d.attachments {
            a.saved = None;
        }
        return Ok(Stored::Deleted(db.delete_draft(row)?));
    }
    let unsaved = d.attachments.iter().any(|a| a.saved.is_none());
    if d.row.is_some() && !d.dirty && !unsaved {
        return Ok(Stored::Nothing);
    }
    if d.message_id.is_empty() {
        d.message_id = mail::smtp::message_id(email);
    }
    let (row, stored) = match d.row {
        Some(row) if !d.dirty || db.update_draft(&d.to_row(row))? => (row, Stored::Updated),
        // The saved row is gone: save the draft again, copies included.
        Some(_) => {
            for a in &mut d.attachments {
                a.saved = None;
            }
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
    for a in d.attachments.iter_mut().filter(|a| a.saved.is_none()) {
        let mime = mail::smtp::content_type(&a.file.filename);
        a.saved = Some(
            db.add_draft_file(row, &a.file.filename, mime, &a.file.data)?
                .id,
        );
    }
    Ok(stored)
}

/// Load saved draft `row` with its attachments. Copies that can't be read
/// are left out (and stay saved); their names come back.
pub(super) fn load(db: &Db, row: i64) -> anyhow::Result<Option<(Draft, Vec<String>)>> {
    let Some(r) = db.draft(row)? else {
        return Ok(None);
    };
    let mut attachments = Vec::new();
    let mut missing = Vec::new();
    let mut total = 0;
    for f in db.draft_files(row)? {
        let limit = (MAX_ATTACH_HARD * 3 / 4).saturating_sub(total);
        match db.read_draft_file(row, &f, limit) {
            Ok(data) => {
                total += data.len() as u64;
                attachments.push(Attached {
                    file: OutAttachment {
                        filename: f.name,
                        data,
                    },
                    saved: Some(f.id),
                });
            }
            Err(_) => missing.push(f.name),
        }
    }
    Ok(Some((Draft::from_row(r, attachments), missing)))
}

/// Whether an autosave also copies its draft to the server, the last copy
/// having been asked for at `last`: at most one per `SERVER_SAVE_EVERY`.
fn server_save_due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|last| now.saturating_duration_since(last) >= SERVER_SAVE_EVERY)
}

/// How much longer an autosave waits when it comes due: edits less than
/// `AUTOSAVE_AFTER` before `now` push it back (`None`: save now). So edits
/// in quick succession end in one save, two seconds after the last.
fn autosave_wait(last_edit: Instant, now: Instant) -> Option<Duration> {
    AUTOSAVE_AFTER
        .checked_sub(now.saturating_duration_since(last_edit))
        .filter(|d| !d.is_zero())
}

impl App {
    /// The message actions (reply, read, delete, move, next/previous) run
    /// only while the reader shows: never over the composer or Settings.
    pub(super) fn reading(&self) -> bool {
        let ui = self.ui();
        !ui.get_composing() && !ui.get_show_settings() && !self.tab_gone()
    }

    /// The account a new message comes from: the open folder's, or in All
    /// inboxes the open message's.
    fn current_account(&self) -> Option<i64> {
        let id = match self.view {
            View::Folder(f) => self
                .folders
                .iter()
                .find(|x| x.id == f)
                .map(|x| x.account_id),
            View::AllInboxes | View::Drafts | View::Outbox => {
                self.open.as_ref().map(|o| o.msg.account_id)
            }
        };
        id.or_else(|| self.accounts.first().map(|a| a.id))
    }

    /// New mail: always a blank message (UX plan "Drafts"). A draft in
    /// progress, shown or parked, is saved and stays in Drafts.
    pub(super) fn compose_new(&mut self) {
        let Some(account) = self.current_account() else {
            return;
        };
        self.start_draft(Draft::new(account));
    }

    /// A `mailto:` link was clicked: a new message with its fields, from
    /// the open message's account. A draft in progress is kept.
    pub(super) fn compose_mailto(&mut self, m: Mailto) {
        let Some(account) = self.current_account() else {
            return self.set_status(&t!(invoke_add_account_write), false);
        };
        let account = self.open.as_ref().map_or(account, |o| o.msg.account_id);
        self.start_draft(Draft {
            show_cc: !m.cc.is_empty() || !m.bcc.is_empty(),
            to: m.to,
            cc: m.cc,
            bcc: m.bcc,
            subject: m.subject,
            body: m.body,
            ..Draft::new(account)
        });
    }

    pub(super) fn reply(&mut self, all: bool) {
        if !self.reading() {
            return;
        }
        let Some(open) = &self.open else { return };
        let (msg, body) = (open.msg.clone(), open.body.clone());
        self.reply_to(&msg, body.as_ref(), all);
    }

    /// Reply (or Reply all) to `msg`, whose parsed body is `body` (none:
    /// still downloading): the open message, or a message window's.
    pub(super) fn reply_to(&mut self, msg: &db::Message, body: Option<&Body>, all: bool) {
        // A draft in the server's Drafts folder is not mail to answer (its
        // InfoBar offers Forward instead; editing it in place is deferred).
        if self.folder_role(msg.folder_id) == Role::Drafts {
            return;
        }
        let Some(body) = body else {
            return self.set_status(&t!(invoke_still_downloading), false);
        };
        // A message replied to already: its draft comes back.
        let message = msg.id;
        if let Some(id) = self.drafts.find_reply(message) {
            return self.resume_draft(id);
        }
        match self.db.draft_for_message(message) {
            Ok(Some(row)) => return self.open_saved(row),
            Ok(None) => {}
            Err(e) => return self.set_status(&text::detail(&e), true),
        }
        let me = self.account_email(msg.account_id);
        let heads = Heads::shown(self.lang);
        let mut draft = Draft::reply(msg, body, &me, all, &heads);
        if let Some(html) = quotable(msg.raw.as_deref()) {
            draft.quote_html(msg, &html, &heads);
        }
        draft.quote_text(msg, strip_signature(&body.text), &heads);
        self.start_draft(draft);
    }

    pub(super) fn forward(&mut self) {
        if !self.reading() {
            return;
        }
        let Some(open) = &self.open else { return };
        let (msg, body) = (open.msg.clone(), open.body.clone());
        self.forward_of(&msg, body.as_ref());
    }

    /// Forward `msg` (the open message, or a message window's).
    pub(super) fn forward_of(&mut self, msg: &db::Message, body: Option<&Body>) {
        let (Some(body), Some(raw)) = (body, msg.raw.as_deref()) else {
            return self.set_status(&t!(invoke_still_downloading), false);
        };
        let attachments = mime::attachments(raw, |_| true)
            .into_iter()
            .map(|(filename, data)| OutAttachment { filename, data }.into())
            .collect();
        let heads = Heads::shown(self.lang);
        let mut draft = Draft::forward(msg, body, attachments, &heads);
        if let Some(html) = quotable(Some(raw)) {
            draft.quote_html(msg, &html, &heads);
        }
        draft.quote_text(msg, &body.text, &heads);
        self.start_draft(draft);
    }

    /// Forward as attachment: the open message, whose body is here.
    pub(super) fn forward_attached(&mut self) {
        let Some(open) = &self.open else { return };
        let Some(raw) = open.msg.raw.clone() else {
            return;
        };
        self.start_draft(Draft::forward_attached(&open.msg, raw));
    }

    /// Make `draft` the active one and show it; a draft in progress is
    /// saved and kept (in a tab of its own when it showed in the reading
    /// pane). From a message or draft tab, the new draft gets a tab of its
    /// own to the right. A new draft gets its account's signature.
    pub(super) fn start_draft(&mut self, mut draft: Draft) {
        self.prepare_draft(&mut draft);
        if !self.in_mail_tab() || self.draft_tab {
            let id = self.drafts.add(draft);
            return self.open_draft_tab(id, false);
        }
        let aside = self.pane_draft_aside();
        let parked = !aside && self.park();
        self.drafts.open(draft);
        self.show_draft();
        if aside {
            self.set_status(&t!(invoke_draft_kept_in_tab), false);
        } else if parked {
            self.set_status(&t!(invoke_earlier_draft_saved), false);
        }
    }

    /// A draft's form as the composer edits it: formatted (unless Settings
    /// say plain text) with its account's signature, the quote folded.
    pub(super) fn prepare_draft(&self, draft: &mut Draft) {
        // A new message starts formatted unless Settings says plain text
        // (a forward as attachment has nothing to format either way).
        if draft.row.is_none() && draft.rich.is_none() && self.compose_rich {
            let sig = self
                .signature_for(draft.account_id, draft.kind)
                .and_then(|s| s.block());
            draft.make_rich(self.compose_defaults, sig);
        }
        if draft.rich.is_none() {
            let block = self.signature_of(draft.account_id, draft.kind);
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

    /// Show draft `id`, already in memory, saving the one it replaces: its
    /// tab comes forward if it has one; from a message or draft tab it gets
    /// a tab of its own.
    fn resume_draft(&mut self, id: DraftId) {
        if let Some(i) = self.tabs.find(tabs::TabKind::Draft(id)) {
            return self.activate_tab(i);
        }
        if !self.in_mail_tab() || self.draft_tab {
            return self.open_draft_tab(id, false);
        }
        if self.drafts.active_id() != Some(id) {
            let aside = self.pane_draft_aside();
            let parked = !aside && self.park();
            self.drafts.switch_to(id);
            if aside {
                self.set_status(&t!(invoke_draft_kept_in_tab), false);
            } else if parked {
                self.set_status(&t!(invoke_earlier_draft_saved), false);
            }
        }
        self.show_draft();
    }

    /// Open saved draft `row` in the composer (from the Drafts view, or a
    /// reply to its message).
    pub(super) fn open_saved(&mut self, row: i64) {
        if let Some(id) = self.drafts.find_row(row) {
            return self.resume_draft(id);
        }
        if let Some(i) = self.tabs.find(tabs::TabKind::Saved(row)) {
            return self.activate_tab(i);
        }
        match self.db.is_saved_draft(row) {
            Ok(true) => {}
            Ok(false) => {
                self.drafts_changed(None, true);
                return self.set_status(&t!(invoke_draft_gone), true);
            }
            Err(e) => {
                return self.set_status(
                    &t!(invoke_could_not_open_draft, format!("{e:#}").into()),
                    true,
                );
            }
        }
        match load(&self.db, row) {
            Ok(Some((draft, missing))) => {
                self.start_draft(draft);
                if !missing.is_empty() {
                    let names = missing.join(", ");
                    self.set_status(&t!(invoke_draft_left_out, names.into()), true);
                }
            }
            Ok(None) => {
                self.drafts_changed(None, true);
                self.set_status(&t!(invoke_draft_gone), true);
            }
            Err(e) => self.set_status(
                &t!(invoke_could_not_open_draft, format!("{e:#}").into()),
                true,
            ),
        }
    }

    /// After start-up's recovery of the outbox: count the saved drafts and
    /// the outbox again for the pane (both are counted before the first
    /// frame too). Drafts are never offered on a status line: the Drafts
    /// entry, always in the pane, holds them.
    pub(super) fn count_drafts_and_outbox(&mut self) {
        self.refresh_draft_count();
        self.refresh_outbox_count();
        self.rebuild_nav();
    }

    pub(super) fn refresh_draft_count(&mut self) {
        match self.db.count_drafts() {
            Ok(n) => self.draft_count = n,
            Err(e) => self.set_status(&text::detail(&e), true),
        }
    }

    /// A saved draft came or went (`count`) or changed: refresh the nav
    /// count, the Draft tag of the message it replies to, and the Drafts
    /// view.
    pub(super) fn drafts_changed(&mut self, reply_to: Option<i64>, count: bool) {
        if count {
            self.refresh_draft_count();
            self.rebuild_nav();
        }
        if let Some(m) = reply_to {
            self.rows.changed(m);
            if self.open.as_ref().is_some_and(|o| o.msg.id == m) {
                self.show_draft_reply();
            }
        }
        if self.view == View::Drafts {
            self.reload_rows();
        }
    }

    /// The reader's "You started a reply to this message" bar, with
    /// Resume: shown while the open message has a reply in progress (in
    /// memory, or saved in Drafts).
    pub(super) fn show_draft_reply(&self) {
        let started = self.open.as_ref().is_some_and(|o| {
            self.drafts.find_reply(o.msg.id).is_some()
                || self.db.draft_for_message(o.msg.id).ok().flatten().is_some()
        });
        self.ui().set_r_draft_reply(started);
    }

    /// Save draft `id` now. Returns false (with the error shown) if that
    /// failed; the draft stays in memory either way.
    pub(super) fn save_draft(&mut self, id: DraftId) -> bool {
        let Some(account) = self.drafts.get(id).map(|d| d.account_id) else {
            return true;
        };
        let email = self.account_email(account);
        let db = self.db.clone();
        let Some(d) = self.drafts.get_mut(id) else {
            return true;
        };
        let reply_to = d.reply_to;
        let stored = store(&db, d, &email);
        if matches!(
            stored,
            Ok(Stored::Inserted | Stored::Updated | Stored::Deleted(_))
        ) && self.drafts.active_id() == Some(id)
        {
            self.show_saved_at();
        }
        match stored {
            Ok(Stored::Nothing) => true,
            Ok(Stored::Updated) => {
                self.drafts_changed(None, false);
                true
            }
            Ok(Stored::Inserted) => {
                self.drafts_changed(reply_to, true);
                true
            }
            Ok(Stored::Deleted(copy)) => {
                self.remove_server_copy(copy);
                self.drafts_changed(reply_to, true);
                true
            }
            Err(e) => {
                self.set_status(
                    &t!(invoke_could_not_save_draft, format!("{e:#}").into()),
                    true,
                );
                false
            }
        }
    }

    /// Have the sync worker copy draft `id`'s saved row to the server's
    /// Drafts folder; unless `now`, only if the last copy was asked for
    /// `SERVER_SAVE_EVERY` ago. The worker skips a row unchanged since.
    fn save_to_server(&mut self, id: DraftId, now: bool) {
        let Some((row, account)) = self
            .drafts
            .get(id)
            .and_then(|d| Some((d.row?, d.account_id)))
        else {
            return;
        };
        let at = Instant::now();
        if !now && !server_save_due(self.server_saves.get(&row).copied(), at) {
            return;
        }
        self.server_saves.insert(row, at);
        if let Some(w) = self.workers.get(&account) {
            w.send(Job::SaveDraft { row });
        }
    }

    /// [`App::save_to_server`] at once (a draft leaving the composer).
    pub(super) fn save_to_server_now(&mut self, id: DraftId) {
        self.save_to_server(id, true);
    }

    /// Have the owning account's sync worker remove a draft's server copy.
    pub(super) fn remove_server_copy(&self, copy: Option<db::ServerCopy>) {
        if let Some(copy) = copy
            && let Some(w) = self.workers.get(&copy.account_id)
        {
            w.send(Job::DeleteDraft(copy));
        }
    }

    /// Save every changed draft (the window is closing).
    pub(super) fn save_all_drafts(&mut self) {
        for id in self.drafts.ids() {
            self.save_draft(id);
        }
    }

    /// Save the active draft before another one takes its place. True if
    /// it was saved and stays.
    fn park(&mut self) -> bool {
        let Some(id) = self.drafts.active_id() else {
            return false;
        };
        let kept = self.save_draft(id) && self.drafts.get(id).is_some_and(Draft::keep);
        if kept {
            self.save_to_server(id, true);
        }
        kept
    }

    /// An edit: save the active draft two seconds after the last one.
    pub(super) fn schedule_autosave(&mut self) {
        self.last_edit = Instant::now();
        if !self.autosave_timer.running() {
            self.autosave_timer
                .start(TimerMode::SingleShot, AUTOSAVE_AFTER, || {
                    with_app(|a| a.autosave());
                });
        }
    }

    fn autosave(&mut self) {
        if let Some(rest) = autosave_wait(self.last_edit, Instant::now()) {
            self.autosave_timer.start(TimerMode::SingleShot, rest, || {
                with_app(|a| a.autosave());
            });
            return;
        }
        if let Some(id) = self.drafts.active_id()
            && self.save_draft(id)
        {
            self.save_to_server(id, false);
        }
    }

    /// Push the active draft into the composer and show it, focusing To
    /// for a message without recipients and otherwise the top of the body.
    pub(super) fn show_draft(&mut self) {
        let ui = self.ui();
        let accounts: Vec<i64> = self.accounts.iter().map(|a| a.id).collect();
        let Some(d) = self.drafts.active_mut() else {
            return;
        };
        // The account may have been removed since; fall back to the first.
        let index = match accounts.iter().position(|id| *id == d.account_id) {
            Some(i) => i,
            None => {
                d.account_id = accounts.first().copied().unwrap_or_default();
                0
            }
        };
        ui.set_c_title(d.kind.title().into());
        ui.set_c_from_index(index as i32);
        ui.set_c_to(d.to.as_str().into());
        ui.set_c_cc(d.cc.as_str().into());
        ui.set_c_bcc(d.bcc.as_str().into());
        ui.set_c_show_cc(d.show_cc);
        ui.set_c_subject(d.subject.as_str().into());
        ui.set_c_body(d.body.as_str().into());
        let formatted = d.rich.is_some();
        ui.set_c_rich(formatted);
        ui.set_c_quote_folded(match &d.rich {
            Some(ed) => ed.has_quote() && ed.quote_folded(),
            None => d.quote.is_some(),
        });
        ui.set_c_importance(d.importance.code() as i32);
        ui.set_c_preview(false);
        ui.set_c_guard(SharedString::new());
        self.drafts.suggest = None;
        ui.set_c_suggestions(ModelRc::default());
        let Some(d) = self.drafts.active() else {
            return;
        };
        let focus = if d.to.trim().is_empty() {
            ComposeField::To
        } else {
            ComposeField::Body
        };
        self.show_send_state();
        self.show_compose_attachments();
        self.show_saved_at();
        self.ed_reset(rich::BODY);
        if formatted {
            self.ed_show(rich::BODY, true, true, false);
        }
        self.show_signature_choices();
        self.focus_field(focus);
        ui.set_composing(true);
        self.sync_selection_index();
    }

    /// The header caption: "Draft saved 14:05" once the active draft is
    /// saved, else nothing.
    fn show_saved_at(&self) {
        let at = self.drafts.active().and_then(|d| d.saved_at);
        let text = at.map_or_else(String::new, |t| {
            t!(invoke_draft_saved_at, format::time(t).into())
        });
        self.ui().set_c_saved(text.into());
    }

    /// "Show quoted text": the folded quote goes back into the body,
    /// unchanged (nothing to save).
    pub(super) fn unfold_quote(&mut self) {
        let Some(d) = self.drafts.active_mut() else {
            return;
        };
        if let Some(ed) = d.rich.as_mut() {
            if ed.fold_quote(false) {
                self.ui().set_c_quote_folded(false);
                self.ed_show(rich::BODY, true, true, false);
            }
            return;
        }
        if d.unfold_quote() {
            let body = SharedString::from(d.body.as_str());
            let ui = self.ui();
            ui.set_c_body(body);
            ui.set_c_quote_folded(false);
        }
    }

    /// The composer's texts, again in the language shown.
    pub(super) fn repush_composer(&mut self) {
        let ui = self.ui();
        let Some(d) = self.drafts.active() else {
            return;
        };
        if !ui.get_composing() {
            return;
        }
        ui.set_c_title(d.kind.title().into());
        self.show_saved_at();
        // The bar of the send guards asks the same question, if it is up.
        if !ui.get_c_guard().is_empty() {
            let guard = send_guard(d).map(text::guard).unwrap_or_default();
            ui.set_c_guard(guard.into());
        }
        self.show_send_state();
        self.show_compose_attachments();
    }

    /// A formatting button or key on the body's selection.
    pub(super) fn format_body(
        &mut self,
        action: FormatAction,
        anchor: i32,
        cursor: i32,
    ) -> BodyEdit {
        use markup::{BOLD, ITALIC, Prefix};
        self.edit_body(anchor, cursor, |text, a, c| match action {
            FormatAction::Bold => markup::wrap_selection(text, a, c, BOLD),
            FormatAction::Italic => markup::wrap_selection(text, a, c, ITALIC),
            FormatAction::Bullets => markup::toggle_line_prefix(text, a, c, Prefix::Bullet),
            FormatAction::Numbers => markup::toggle_line_prefix(text, a, c, Prefix::Number),
            FormatAction::Quote => markup::toggle_line_prefix(text, a, c, Prefix::Quote),
        })
    }

    /// The link field's address, on the selection it was opened with.
    pub(super) fn link_body(&mut self, url: &str, anchor: i32, cursor: i32) -> BodyEdit {
        self.edit_body(anchor, cursor, |text, a, c| {
            markup::insert_link(text, a, c, url)
        })
    }

    /// Apply a formatting edit to the active draft's body and show it.
    fn edit_body(
        &mut self,
        anchor: i32,
        cursor: i32,
        edit: impl FnOnce(&str, usize, usize) -> Option<markup::Edited>,
    ) -> BodyEdit {
        let (Ok(a), Ok(c)) = (usize::try_from(anchor), usize::try_from(cursor)) else {
            return BodyEdit::default();
        };
        let Some(d) = self.drafts.active() else {
            return BodyEdit::default();
        };
        let Some((after, new_a, new_c)) = edit(&d.body, a, c) else {
            return BodyEdit::default();
        };
        let before = d.body.clone();
        self.draft_edited(ComposeField::Body, &after);
        self.ui().set_c_body(after.as_str().into());
        self.first_format();
        if let Some(d) = self.drafts.active_mut() {
            d.unformat = Some(Unformat {
                before,
                after,
                anchor: a,
                cursor: c,
            });
        }
        body_edit(new_a, new_c)
    }

    /// The first formatting edit ever: a caption under the formatting row
    /// says what the marks do (once; saved as seen as it shows).
    fn first_format(&mut self) {
        if self.format_hint_seen {
            return;
        }
        self.format_hint_seen = true;
        self.ui().set_c_format_hint(true);
        if let Err(e) = self.db.set_setting("format_hint_seen", "1") {
            self.set_status(&text::detail(&e), true);
        }
    }

    /// Preview on (or the composer resized while on): render what the
    /// recipient gets, the draft's HTML part, `width` logical px wide, on
    /// a short-lived thread; the newest render wins.
    /// `first`: Preview just turned on, so it is drawn at once (the last
    /// preview's picture goes first); a new width or theme waits for the
    /// resize to settle (`PREVIEW_DEBOUNCE`), one render for the lot.
    pub(super) fn render_preview(&mut self, width: f32, dark: bool, first: bool) {
        self.preview_want = (width, dark);
        if first {
            self.preview_timer.stop();
            let ui = self.ui();
            ui.set_c_preview_image(slint::Image::default());
            ui.set_c_preview_height(0.0);
            ui.set_c_preview_cut(false);
            ui.set_c_preview_failed(false);
            return self.start_preview();
        }
        self.preview_timer
            .start(TimerMode::SingleShot, PREVIEW_DEBOUNCE, || {
                with_app(|a| a.start_preview());
            });
    }

    fn start_preview(&mut self) {
        let ui = self.ui();
        if !ui.get_c_preview() {
            return;
        }
        let Some(d) = self.drafts.active() else {
            return;
        };
        let html = match &d.rich {
            Some(ed) if model::needs_html(ed.doc()) => model::html(ed.doc(), model::Mode::Send),
            Some(ed) => mail::compose_html::preview_html(&model::plain(ed.doc()), None),
            None => format!(
                "<div style=\"white-space:pre-wrap\">{}</div>",
                text_html(&d.full_body())
            ),
        };
        let (width, dark) = self.preview_want;
        let scale = ui.window().scale_factor();
        self.preview_seq += 1;
        let seq = self.preview_seq;
        let render = move || {
            let page = preview_page(html, width, scale, dark);
            on_ui(Box::new(move |a| a.show_preview(seq, page)));
        };
        // Layout recurses per nesting level: the reader worker's stack.
        let spawned = std::thread::Builder::new()
            .name("preview".into())
            .stack_size(PREVIEW_STACK)
            .spawn(render);
        if let Err(e) = spawned {
            self.set_status(&text::detail(&anyhow::Error::from(e)), true);
        }
    }

    fn show_preview(&mut self, seq: u64, page: Result<PreviewPage, render::RenderError>) {
        let ui = self.ui();
        if seq != self.preview_seq || !ui.get_c_preview() {
            return;
        }
        match page {
            Ok(p) => {
                let [r, g, b, _] = p.bg;
                ui.set_c_preview_paper(slint::Color::from_rgb_u8(r, g, b));
                ui.set_c_preview_height(p.height);
                ui.set_c_preview_width(p.width);
                ui.set_c_preview_image(slint::Image::from_rgba8_premultiplied(p.pixels));
                ui.set_c_preview_cut(p.cut);
                ui.set_c_preview_failed(false);
            }
            Err(_) => {
                ui.set_c_preview_image(slint::Image::default());
                ui.set_c_preview_failed(true);
            }
        }
    }

    /// Ctrl+Z right after a formatting edit: the body as it was.
    pub(super) fn unformat_body(&mut self) -> BodyEdit {
        let Some(d) = self.drafts.active_mut() else {
            return BodyEdit::default();
        };
        // Right after a switch to plain text: the formatted message back.
        if d.unplain.as_ref().is_some_and(|(_, text)| *text == d.body) {
            self.set_rich(true);
            return BodyEdit::default();
        }
        let Some(u) = d.unformat.take_if(|u| u.after == d.body) else {
            return BodyEdit::default();
        };
        self.draft_edited(ComposeField::Body, &u.before);
        self.ui().set_c_body(u.before.as_str().into());
        body_edit(u.anchor, u.cursor)
    }

    /// The keyboard focus to the body, plain or formatted.
    pub(super) fn focus_body(&self) {
        self.focus_field(ComposeField::Body);
    }

    /// Move the keyboard focus to a composer field.
    fn focus_field(&self, field: ComposeField) {
        let ui = self.ui();
        ui.set_c_focus(field);
        ui.set_c_focus_seq(ui.get_c_focus_seq().wrapping_add(1));
    }

    /// The inline address errors and why Send is off.
    fn show_send_state(&self) {
        let ui = self.ui();
        let (errors, block, need) = match self.drafts.active() {
            Some(d) => (d.errors.clone(), d.send_block(), d.need_recipient),
            None => Default::default(),
        };
        let [to, cc, bcc] = errors.map(|bad| bad.map(|a| text::address_invalid(&a)));
        let to = to.or_else(|| need.then(|| t!(invoke_add_recipient)));
        ui.set_c_to_error(to.unwrap_or_default().into());
        ui.set_c_cc_error(cc.unwrap_or_default().into());
        ui.set_c_bcc_error(bcc.unwrap_or_default().into());
        ui.set_c_send_reason(block.map(text::send_block).unwrap_or_default().into());
    }

    pub(super) fn show_compose_attachments(&self) {
        let rows: Vec<AttachmentRow> = self
            .drafts
            .active()
            .map(|d| d.attachments.as_slice())
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(i, a)| AttachmentRow {
                index: i as i32,
                name: a.file.filename.as_str().into(),
                size: format::size(self.lang, a.file.data.len()).into(),
                kind: format::file_kind(&a.file.filename),
            })
            .collect();
        let encoded = self
            .drafts
            .active()
            .map_or(0, |d| encoded_size(attachments_size(&d.attachments)));
        let ui = self.ui();
        ui.set_c_attachments(ModelRc::new(VecModel::from(rows)));
        ui.set_c_attach_total(
            t!(
                invoke_attach_total,
                format::size(self.lang, usize::try_from(encoded).unwrap_or(usize::MAX)).into()
            )
            .into(),
        );
        ui.set_c_attach_level(counter_level(encoded));
    }

    /// The user typed into a composer field.
    pub(super) fn draft_edited(&mut self, field: ComposeField, text: &str) {
        let Some(d) = self.drafts.active_mut() else {
            return;
        };
        let cursor = caret_after_edit(d.field(field), text);
        d.edit(field, text);
        if address_index(field).is_some() {
            self.show_send_state();
            self.suggest(field, cursor);
        }
        // The guard bar asked about the draft as it was.
        self.ui().set_c_guard(SharedString::new());
        self.schedule_autosave();
        if field == ComposeField::Subject {
            self.retitle_draft_tab();
        }
    }

    /// Set a field of the active draft and show it (as if typed). A
    /// formatted body gets `text` at its start.
    pub(super) fn set_draft_field(&mut self, field: ComposeField, text: &str) {
        if field == ComposeField::Body
            && let Some(d) = self.drafts.active_mut()
            && let Some(ed) = d.rich.as_mut()
        {
            if let Some(caret) = ed.replace(0, 0, text, (0, 0)) {
                d.touched = true;
                d.dirty = true;
                self.ed_set_caret(rich::BODY, caret);
                self.schedule_autosave();
            }
            self.ed_show(rich::BODY, true, true, false);
            return;
        }
        self.draft_edited(field, text);
        self.show_field(field, text);
    }

    /// Put `text` in a composer field (the draft is not changed).
    fn show_field(&self, field: ComposeField, text: &str) {
        let ui = self.ui();
        let text = SharedString::from(text);
        match field {
            ComposeField::To => ui.set_c_to(text),
            ComposeField::Cc => ui.set_c_cc(text),
            ComposeField::Bcc => ui.set_c_bcc(text),
            ComposeField::Subject => ui.set_c_subject(text),
            ComposeField::Body => ui.set_c_body(text),
        }
    }

    /// Suggest recipients for the address at `cursor` in `field`, from the
    /// draft account's address index: once it has [`SUGGEST_AFTER`]
    /// characters, leaving out addresses the draft already has. The query
    /// runs on the UI thread (it reads one account's index).
    fn suggest(&mut self, field: ComposeField, cursor: usize) {
        let items = self.drafts.active().map_or_else(Vec::new, |d| {
            let typed = typed_address(d.field(field), cursor);
            if typed.chars().count() < SUGGEST_AFTER {
                return Vec::new();
            }
            let taken: HashSet<String> = [&d.to, &d.cc, &d.bcc]
                .into_iter()
                .flat_map(|list| mail::smtp::split_addresses(list))
                .map(|entry| mail::smtp::mailbox_parts(&entry).1.to_lowercase())
                .collect();
            let found = self
                .db
                .suggest_addresses(d.account_id, typed, SUGGESTIONS + taken.len())
                .unwrap_or_default();
            found
                .into_iter()
                .filter(|r| !taken.contains(&r.addr))
                .take(SUGGESTIONS)
                .collect()
        });
        let shown = (!items.is_empty()).then_some(Suggest {
            field,
            cursor,
            items,
        });
        self.show_suggestions(shown);
    }

    /// Show `suggest` under its field, the first one highlighted (`None`:
    /// hide them).
    fn show_suggestions(&mut self, suggest: Option<Suggest>) {
        let ui = self.ui();
        let rows: Vec<Suggestion> = suggest
            .iter()
            .flat_map(|s| &s.items)
            .map(|r| Suggestion {
                name: r.name.as_str().into(),
                addr: r.addr.as_str().into(),
            })
            .collect();
        ui.set_c_suggest_index(0);
        ui.set_c_suggestions(ModelRc::new(VecModel::from(rows)));
        self.drafts.suggest = suggest;
    }

    /// Esc, or the caret moved: hide the suggestions.
    pub(super) fn close_suggestions(&mut self) {
        if self.drafts.suggest.is_some() {
            self.show_suggestions(None);
        }
    }

    /// Take suggestion `index`: it replaces the address being typed, as
    /// `Name <addr>, `, and the caret goes after it.
    pub(super) fn accept_suggestion(&mut self, index: i32) {
        let Some(s) = self.drafts.suggest.take() else {
            return;
        };
        self.show_suggestions(None);
        let Some(r) = usize::try_from(index).ok().and_then(|i| s.items.get(i)) else {
            return;
        };
        let Some(d) = self.drafts.active() else {
            return;
        };
        let mailbox = mail::smtp::mailbox(&r.name, &r.addr);
        let (text, caret) = complete_address(d.field(s.field), s.cursor, &mailbox);
        self.show_field(s.field, &text);
        self.draft_edited(s.field, &text);
        let ui = self.ui();
        ui.set_c_focus(s.field);
        ui.set_c_caret(i32::try_from(caret).unwrap_or(i32::MAX));
        ui.set_c_caret_seq(ui.get_c_caret_seq().wrapping_add(1));
    }

    /// The focus left a composer field: check its addresses.
    pub(super) fn draft_left(&mut self, field: ComposeField) {
        self.close_suggestions();
        if let Some(d) = self.drafts.active_mut() {
            d.validate(field);
            self.show_send_state();
        }
    }

    /// Another From account: its signature replaces the old one if that is
    /// untouched, and the draft is saved at once unless it is blank.
    pub(super) fn draft_account(&mut self, index: i32) {
        let Some(account) = self.accounts.get(index.max(0) as usize) else {
            return;
        };
        let id = account.id;
        let Some(kind) = self.drafts.active().map(|d| d.kind) else {
            return;
        };
        let sig = self.signature_for(id, kind);
        let block = sig
            .as_ref()
            .map_or_else(String::new, |s| signature_block(&s.body_text));
        let Some(d) = self.drafts.active_mut() else {
            return;
        };
        if d.account_id == id {
            return;
        }
        d.account_id = id;
        // A blank message stays untouched: switching From is not writing,
        // so Esc still drops it silently (a reply keeps the choice).
        d.touched |= !d.is_empty();
        d.dirty = true;
        if let Some(ed) = d.rich.as_mut() {
            // The new account's signature, while the one there is as the
            // app put it in.
            let untouched = ed.signature().is_none_or(|s| s.is_untouched());
            let new = sig.and_then(|s| s.block());
            let same = ed.signature().map(|s| s.source) == new.as_ref().map(|s| s.source);
            if untouched && !same && ed.set_signature(new) {
                self.ed_show(rich::BODY, true, true, false);
                self.show_signature_choices();
            }
        } else if d.resign(&block) {
            let body = SharedString::from(d.body.as_str());
            self.ui().set_c_body(body);
        }
        if let Some(draft) = self.drafts.active_id() {
            self.save_draft(draft);
        }
    }

    /// The signature an account puts in a new message, or in a reply or
    /// forward (none: it has none, or the cache could not be read).
    pub(super) fn signature_for(&self, account: i64, kind: Kind) -> Option<db::Signature> {
        self.db
            .default_signature(account, kind != Kind::New)
            .ok()
            .flatten()
    }

    /// The plain-text signature block of an account's mail of `kind`
    /// (empty: none).
    fn signature_of(&self, account: i64, kind: Kind) -> String {
        self.signature_for(account, kind)
            .map_or_else(String::new, |s| signature_block(&s.body_text))
    }

    /// The formatting bar's High or Low importance toggle: `code` is the
    /// importance picked (`Importance::code`; 0 when the one on is turned
    /// off). The draft is saved at once, like another From account, unless
    /// it is blank ([`Draft::is_empty`]).
    pub(super) fn draft_importance(&mut self, code: i32) {
        let importance = db::Importance::from_code(code.into());
        self.ui().set_c_importance(importance.code() as i32);
        let Some(d) = self.drafts.active_mut() else {
            return;
        };
        if d.importance == importance {
            return;
        }
        d.importance = importance;
        // As for another From account: on a blank message (no recipients,
        // subject, body or attachments) importance alone keeps nothing, so
        // it is not saved here or copied to the server's Drafts; on a
        // reply it is a choice worth keeping.
        d.touched |= !d.is_empty();
        d.dirty = true;
        if let Some(draft) = self.drafts.active_id() {
            self.save_draft(draft);
        }
    }

    /// Show the Cc and Bcc rows and focus `field` (Cc or Bcc).
    pub(super) fn reveal_cc(&mut self, field: ComposeField) {
        if let Some(d) = self.drafts.active_mut() {
            d.show_cc = true;
        }
        self.ui().set_c_show_cc(true);
        self.focus_field(field);
    }

    /// Remove an attachment, and its saved copy at once.
    pub(super) fn remove_attachment(&mut self, index: i32) {
        let db = self.db.clone();
        let Some(id) = self.drafts.active_id() else {
            return;
        };
        if let Some(d) = self.drafts.get_mut(id)
            && let Ok(i) = usize::try_from(index)
            && i < d.attachments.len()
        {
            let gone = d.attachments.remove(i);
            d.touched = true;
            d.dirty = true;
            if let (Some(row), Some(file)) = (d.row, gone.saved)
                && let Err(e) = db.remove_draft_file(row, file)
            {
                self.set_status(&text::detail(&e), true);
            }
            self.show_compose_attachments();
            self.show_send_state();
            self.save_draft(id);
        }
    }

    /// Esc or Close: save the draft and hide the composer (see
    /// [`App::park_composer`]); in a composer tab, close the tab.
    pub(super) fn close_composer(&mut self) {
        if !self.in_mail_tab() {
            let i = self.tabs.active();
            return self.close_tab(i);
        }
        self.park_composer();
        self.hide_composer();
    }

    /// Every way out of the composer that keeps the draft (Esc, Close, a
    /// click on a message or an Outbox row) parks it the same way: a draft
    /// worth keeping is saved here and copied to the server's Drafts at
    /// once, and the status line says "Draft saved"; an empty one goes.
    pub(super) fn park_composer(&mut self) {
        if let Some(id) = self.drafts.active_id() {
            let saved = self.save_draft(id);
            if !self.drafts.get(id).is_some_and(Draft::keep) {
                self.drafts.remove(id);
            } else if saved {
                self.save_to_server(id, true);
                self.set_status(&t!(invoke_draft_saved), false);
            }
        }
    }

    fn hide_composer(&mut self) {
        // Sent or discarded in a composer tab: the tab goes with it.
        if let tabs::TabKind::Draft(id) = self.tabs.active_kind()
            && !self.drafts.contains(id)
        {
            return self.drop_active_tab();
        }
        let ui = self.ui();
        ui.set_c_suggestions(ModelRc::default());
        ui.set_composing(false);
        ui.invoke_focus_list();
        self.sync_selection_index();
        self.show_draft_reply();
    }

    /// Discard: a draft with something the user wrote asks first.
    pub(super) fn discard(&mut self) {
        match self.drafts.active() {
            Some(d) if d.keep() => self.ui().set_c_confirm_discard(true),
            _ => self.discard_confirmed(),
        }
    }

    /// Drop the active draft with its saved row and attachment copies.
    pub(super) fn discard_confirmed(&mut self) {
        self.ui().set_c_confirm_discard(false);
        if let Some(id) = self.drafts.active_id()
            && let Some(d) = self.drafts.remove(id)
            && let Some(row) = d.row
        {
            match self.db.delete_draft(row) {
                Ok(copy) => self.remove_server_copy(copy),
                Err(e) => self.set_status(&text::detail(&e), true),
            }
            self.drafts_changed(d.reply_to, true);
        }
        self.hide_composer();
    }

    /// Send (Ctrl+Enter, Alt+S): the send guards may ask first.
    pub(super) fn send(&mut self) {
        self.send_draft(false);
    }

    /// "Send anyway" on the guard bar.
    pub(super) fn send_anyway(&mut self) {
        self.send_draft(true);
    }

    /// Send the active draft: it is saved as it is, moves to the outbox
    /// (frozen: no more autosaves) and the composer closes. Its message is
    /// built on a worker while the undo window runs.
    fn send_draft(&mut self, anyway: bool) {
        let Some(id) = self.drafts.active_id() else {
            return;
        };
        let Some(d) = self.drafts.get_mut(id) else {
            return;
        };
        for field in ADDRESS_FIELDS {
            d.validate(field);
        }
        let block = d.send_block();
        // Send is always on: pressed too early, the field at fault says
        // why (inline, announced) and takes the focus.
        match block {
            Some(SendBlock::FixAddress(field)) => {
                self.show_send_state();
                return self.focus_field(field);
            }
            Some(SendBlock::NoRecipient) => {
                d.need_recipient = true;
                self.show_send_state();
                return self.focus_field(ComposeField::To);
            }
            Some(block) => {
                self.show_send_state();
                return self.set_status(&text::send_block(block), true);
            }
            None => {}
        }
        let guard = if anyway { None } else { send_guard(d) };
        let account_id = d.account_id;
        self.ui()
            .set_c_guard(guard.map(text::guard).unwrap_or_default().into());
        if guard.is_some() {
            return;
        }
        let Some(account) = self.accounts.iter().find(|a| a.id == account_id).cloned() else {
            return self.set_status(&t!(invoke_add_account_send), true);
        };
        // Saved as it stands, even a reply sent just as it was filled in.
        if let Some(d) = self.drafts.get_mut(id) {
            d.touched = true;
            d.dirty = true;
        }
        if !self.save_draft(id) {
            return;
        }
        let Some(row) = self.drafts.get(id).and_then(|d| d.row) else {
            return;
        };
        match self.db.outbox_send(row) {
            Ok(true) => {}
            Ok(false) => return self.set_status(&t!(invoke_draft_gone), true),
            Err(e) => {
                return self.set_status(&t!(invoke_could_not_send, format!("{e:#}").into()), true);
            }
        }
        let Some(d) = self.drafts.remove(id) else {
            return;
        };
        let reply_to = d.reply_to;
        self.hide_composer();
        self.drafts_changed(reply_to, true);
        self.queue_send(
            row,
            account_id,
            outgoing(&self.db, &account, d),
            self.undo_delay,
        );
    }

    /// Build outbox row `row`'s message on a worker and queue it once
    /// `delay` (the undo window) has passed and the build is done.
    pub(super) fn queue_send(
        &mut self,
        row: i64,
        account: i64,
        outgoing: Outgoing,
        delay: Duration,
    ) {
        self.send_seq += 1;
        let seq = self.send_seq;
        let cancel = Arc::new(AtomicBool::new(false));
        let subject = short_subject(&outgoing.subject);
        self.pending_sends.insert(
            row,
            PendingSend {
                seq,
                account,
                cancel: cancel.clone(),
                built: false,
                due: delay.is_zero(),
                subject: subject.clone(),
            },
        );
        (self.builder)(BuildJob {
            row,
            seq,
            db_path: self.db_path.clone(),
            outgoing,
            cancel,
        });
        if !delay.is_zero() {
            Timer::single_shot(delay, move || {
                with_app(|a| a.send_due(row, seq));
            });
            // The Undo slot at the bottom of the list (a status line never
            // replaces it); it counts down for the whole window, unless a
            // move's offer takes the slot (the send goes on).
            self.offer_undo(
                &t!(
                    invoke_sending_subject_in,
                    subject.into(),
                    count_secs(delay.as_secs())
                ),
                UndoOffer::Send(row),
            );
            let due = Instant::now() + delay;
            self.count_timer
                .start(TimerMode::Repeated, Duration::from_secs(1), move || {
                    with_app(|a| a.count_down(row, due));
                });
        }
        self.outbox_changed();
    }

    /// One second of a send's undo window went: "Sending in 9 s".
    fn count_down(&self, row: i64, due: Instant) {
        if self.undo.get() != UndoOffer::Send(row) {
            return self.count_timer.stop();
        }
        let left = due.saturating_duration_since(Instant::now());
        let secs = left.as_millis().div_ceil(1000);
        let subject = self
            .pending_sends
            .get(&row)
            .map_or_else(String::new, |p| p.subject.clone());
        let text = if secs > 0 {
            t!(
                invoke_sending_subject_in,
                subject.into(),
                count_secs(secs as u64)
            )
        } else {
            // Still being built: it goes once that is done.
            self.count_timer.stop();
            t!(invoke_sending)
        };
        self.update_undo(&text, UndoOffer::Send(row));
    }

    /// The undo window of a send ended.
    fn send_due(&mut self, row: i64, seq: u64) {
        if let Some(p) = self.pending_sends.get_mut(&row)
            && p.seq == seq
        {
            p.due = true;
            if p.built {
                self.promote(row);
            }
        }
    }

    /// A build ended: `Ok(true)` stored the message, `Ok(false)` found the
    /// send undone. A result for a send undone since is dropped.
    pub(super) fn on_built(&mut self, row: i64, seq: u64, result: Result<bool, String>) {
        let Some(p) = self.pending_sends.get_mut(&row) else {
            return;
        };
        if p.seq != seq || p.cancel.load(Ordering::SeqCst) {
            return;
        }
        match result {
            Ok(true) => {
                p.built = true;
                if p.due {
                    self.promote(row);
                }
            }
            Ok(false) => {
                self.pending_sends.remove(&row);
            }
            Err(e) => {
                self.pending_sends.remove(&row);
                if let Err(e) = self.db.outbox_unsend(row) {
                    return self.set_status(&text::detail(&e), true);
                }
                self.set_status(
                    &t!(invoke_could_not_send_back_in_drafts, e.as_str().into()),
                    true,
                );
                self.drafts_changed(None, true);
                self.outbox_changed();
            }
        }
    }

    /// Built and past its undo window: queue the row and hand it to its
    /// account's send worker (offline, it waits in the queue).
    fn promote(&mut self, row: i64) {
        let Some(p) = self.pending_sends.remove(&row) else {
            return;
        };
        self.clear_undo(UndoOffer::Send(row));
        match self.db.outbox_queue(row) {
            Ok(true) => self.dispatch(row, p.account),
            Ok(false) => {}
            Err(e) => self.set_status(&text::detail(&e), true),
        }
        self.outbox_changed();
    }

    pub(super) fn dispatch(&self, row: i64, account: i64) {
        if let Some(w) = self.workers.get(&account) {
            w.send(Job::Send { row });
        }
    }

    /// Undo (or Edit while it waits): the message goes back to Drafts,
    /// unchanged, and opens in the composer. Too late once a send has it.
    pub(super) fn undo_send(&mut self, row: i64) {
        // The flag first: a build finishing now can't store its result.
        if let Some(p) = self.pending_sends.get(&row) {
            p.cancel();
        }
        match self.db.outbox_unsend(row) {
            Ok(true) => {
                self.pending_sends.remove(&row);
                self.clear_undo(UndoOffer::Send(row));
                self.outbox_changed();
                // In a tab of its own, beside what shows now.
                self.open_saved_in_tab(row);
                let reply_to = self.drafts.active().and_then(|d| d.reply_to);
                self.drafts_changed(reply_to, true);
            }
            Ok(false) => {
                self.pending_sends.remove(&row);
                self.clear_undo(UndoOffer::Send(row));
                self.set_status(&t!(invoke_undo_too_late), true);
            }
            // Still cancelled: it is never queued from here.
            Err(e) => self.set_status(&t!(invoke_could_not_undo, format!("{e:#}").into()), true),
        }
    }
}

/// A send in its undo window (outbox row in `Outbox::Pending`).
pub(super) struct PendingSend {
    /// Tells this send apart from a later one of the same row.
    seq: u64,
    pub account: i64,
    /// Set by Undo; the build then stores nothing.
    cancel: Arc<AtomicBool>,
    /// The built message is stored.
    built: bool,
    /// The undo window is over.
    due: bool,
    /// The subject for the offer, shortened (`short_subject`).
    subject: String,
}

/// Subjects longer than this are cut (with an ellipsis) in the undo-send
/// offer, which floats over the list and must stay one or two lines.
const OFFER_SUBJECT_CHARS: usize = 40;

/// `subject` for the undo-send offer: "(no subject)" when empty, cut at
/// [`OFFER_SUBJECT_CHARS`].
fn short_subject(subject: &str) -> String {
    let s = subject.trim();
    if s.is_empty() {
        return t!(invoke_no_subject);
    }
    match s.char_indices().nth(OFFER_SUBJECT_CHARS) {
        Some((i, _)) => format!("{}\u{2026}", s[..i].trim_end()),
        None => s.to_string(),
    }
}

impl PendingSend {
    /// As Undo does: a build finishing now stores nothing.
    pub(super) fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

/// What draft `d` sends from `account`: a formatted draft its text and,
/// when it needs one, an HTML alternative (with the signature logo); a
/// plain-text draft its text only.
pub(super) fn outgoing(db: &Db, account: &Account, d: Draft) -> Outgoing {
    let (body, body_html, inline) = match &d.rich {
        // A logo whose picture is gone is left out (`Db::send_parts`).
        Some(ed) => db.send_parts(ed.doc()).unwrap_or_else(|_| {
            let mut doc = ed.doc().clone();
            for b in &mut doc.blocks {
                if let model::Block::Signature(s) = b {
                    s.logo = None;
                }
            }
            let (text, html) = model::parts(&doc);
            (text, html, Vec::new())
        }),
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

/// The composer's Preview, rendered: the page's pixels (premultiplied, at
/// the device scale), its height in logical px, its canvas colour, and
/// whether it was cut at the longest band one image holds.
pub(super) struct PreviewPage {
    pixels: slint::SharedPixelBuffer<slint::Rgba8Pixel>,
    /// Logical px: the page is never drawn wider than it was laid out.
    width: f32,
    height: f32,
    bg: [u8; 4],
    cut: bool,
}

/// The tallest preview drawn, in device pixels (one raster band).
const PREVIEW_MAX_ROWS: u32 = 4096;
/// A resize with Preview on draws it again once it settles (as the
/// reader's relayout).
const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(150);
/// The preview thread's stack: layout recurses per nesting level (the
/// reader worker's `WORKER_STACK`).
const PREVIEW_STACK: usize = 8 * 1024 * 1024;

/// Lay out and draw `html` (the draft's HTML part) as Preview shows it,
/// under the renderer's caps and deadline. Off the UI thread.
fn preview_page(
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
    let pm = render::rasterize(&doc, &mut engine, 0, rows, scale)
        .ok_or(render::RenderError::TooLarge)?;
    Ok(PreviewPage {
        pixels: slint::SharedPixelBuffer::clone_from_slice(pm.data(), pm.width(), pm.height()),
        width: pm.width() as f32 / scale,
        height: rows as f32 / scale,
        bg: doc.bg,
        cut: full > rows || doc.truncated,
    })
}

/// Build the message of outbox row `row` off the UI thread.
pub(super) struct BuildJob {
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
        let raw = mail::smtp::build_raw(self.outgoing).map_err(|e| format!("{e:#}"))?;
        if self.cancel.load(Ordering::SeqCst) {
            return Ok(false);
        }
        Db::open_worker(&self.db_path)
            .and_then(|db| db.store_raw(self.row, &raw, &self.cancel))
            .map_err(|e| format!("{e:#}"))
    }
}

/// The default builder: a short-lived thread per send.
pub(super) fn spawn_build(job: BuildJob) {
    let build = move || {
        let (row, seq) = (job.row, job.seq);
        let result = job.run();
        on_ui(Box::new(move |a| a.on_built(row, seq, result)));
    };
    background("build", Box::new(build)).expect("spawning build worker");
}

// ------------------------------------------------------- file dialogs
//
// Native dialogs run a nested message loop, so these handlers take what
// they need from the app state, release it, show the dialog, and then
// re-enter.

pub(super) fn attach_files() {
    let Some(target) = with_app(|a| a.drafts.active_id()).flatten() else {
        return;
    };
    let Some(paths) = rfd::FileDialog::new()
        .set_title(t!(invoke_dlg_attach_files))
        .pick_files()
    else {
        return;
    };
    with_app(|a| a.attach_paths(target, paths));
}

impl App {
    /// Attach files dropped on the window or pasted, to the draft showing
    /// (none showing: ignored).
    pub(super) fn attach_dropped(&mut self, paths: Vec<PathBuf>) {
        let ui = self.ui();
        ui.set_file_hover(false);
        if ui.get_show_settings() || paths.is_empty() {
            return;
        }
        // Dropped with no composer open: they go in a new message (the
        // overlay said so).
        if !ui.get_composing() {
            self.compose_new();
            if !ui.get_composing() {
                return;
            }
        }
        if let Some(target) = self.drafts.active_id() {
            self.attach_paths(target, paths);
        }
    }

    /// Ctrl+V in the composer: attach the files copied in Explorer, if the
    /// clipboard holds any (false: the field pastes its text).
    pub(super) fn paste_files(&mut self) -> bool {
        let paths = files::clipboard_files();
        if paths.is_empty() {
            return false;
        }
        self.attach_dropped(paths);
        true
    }

    /// Read `paths` on a short-lived thread and attach them to draft
    /// `target` (the draft showing may change meanwhile).
    pub(super) fn attach_paths(&mut self, target: DraftId, paths: Vec<PathBuf>) {
        let total = self
            .drafts
            .get(target)
            .map_or(0, |d| attachments_size(&d.attachments));
        let read = move || {
            let (added, errors) = read_files(&paths, total);
            on_ui(Box::new(move |a| a.add_attachments(target, added, errors)));
        };
        if let Err(e) = background("attach", Box::new(read)) {
            self.set_status(&t!(invoke_could_not_attach, e.to_string().into()), true);
        }
    }

    /// Add files read for draft `target`, within the hard limit (another
    /// read may have finished first), and save their copies once.
    pub(super) fn add_attachments(
        &mut self,
        target: DraftId,
        added: Vec<Attached>,
        mut errors: Vec<AttachFail>,
    ) {
        let Some(d) = self.drafts.get_mut(target) else {
            return;
        };
        let mut total = attachments_size(&d.attachments);
        let mut any = false;
        for a in added {
            let len = a.file.data.len() as u64;
            if encoded_size(total + len) > MAX_ATTACH_HARD {
                errors.push(AttachFail {
                    name: a.file.filename.clone(),
                    why: AttachWhy::TooBig,
                });
                continue;
            }
            total += len;
            d.attachments.push(a);
            any = true;
        }
        d.touched |= any;
        d.dirty |= any;
        if self.drafts.active_id() == Some(target) {
            self.show_compose_attachments();
            self.show_send_state();
            self.ui().set_c_guard(SharedString::new());
        }
        // The copies are made now, once, not by the autosave.
        if !self.save_draft(target) {
            return;
        }
        if !errors.is_empty() {
            let lines: Vec<String> = errors.iter().map(text::attach_fail).collect();
            self.set_status(&lines.join("; "), true);
        }
    }

    /// Snapshot QA (`compose-attach`): three made-up attachments at 85% of
    /// the counter's 25 MB, never saved.
    pub(super) fn sample_attachments(&mut self) {
        let raw = MAX_ATTACHMENTS_BYTES * 85 / 100 * 3 / 4;
        let files = [
            ("Q3 report.pdf", raw / 2),
            ("Site photos.zip", raw * 3 / 10),
            ("Budget 2027.xlsx", raw - raw / 2 - raw * 3 / 10),
        ];
        if let Some(d) = self.drafts.active_mut() {
            for (name, len) in files {
                d.attachments.push(Attached::from(OutAttachment {
                    filename: name.into(),
                    data: vec![0; len as usize],
                }));
            }
        }
        self.show_compose_attachments();
        self.show_send_state();
    }
}

/// A file that could not be attached, and why.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct AttachFail {
    pub name: String,
    pub why: AttachWhy,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum AttachWhy {
    /// A folder.
    Folder,
    /// Past Komorebi's 100 MB limit.
    TooBig,
    /// The file system's words.
    Io(String),
}

/// Read files to attach when `total` bytes are attached already: the
/// attachments, and what went wrong with each file that can't be.
fn read_files(paths: &[PathBuf], mut total: u64) -> (Vec<Attached>, Vec<AttachFail>) {
    let mut added = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "attachment".into());
        if path.is_dir() {
            errors.push(AttachFail {
                name,
                why: AttachWhy::Folder,
            });
            continue;
        }
        match read_attachment(path, total) {
            Ok(data) => {
                total += data.len() as u64;
                added.push(Attached::from(OutAttachment {
                    filename: name,
                    data,
                }));
            }
            Err(why) => errors.push(AttachFail { name, why }),
        }
    }
    (added, errors)
}

/// The attachment counter's colour: 0 normal, 1 near the limit (from
/// 80%), 2 over it (Send is off).
fn counter_level(encoded: u64) -> i32 {
    if encoded > MAX_ATTACHMENTS_BYTES {
        2
    } else if encoded.saturating_mul(5) >= MAX_ATTACHMENTS_BYTES * 4 {
        1
    } else {
        0
    }
}

fn attachments_size(list: &[Attached]) -> u64 {
    list.iter().map(|a| a.file.data.len() as u64).sum()
}

/// Size once base64-encoded for sending.
fn encoded_size(raw: u64) -> u64 {
    raw.saturating_mul(4) / 3
}

/// Read a file to attach when `total` bytes are attached already. A file that
/// would take the message past the hard limit is refused before reading.
fn read_attachment(path: &Path, total: u64) -> Result<Vec<u8>, AttachWhy> {
    use std::io::Read;
    let io = |e: std::io::Error| AttachWhy::Io(e.to_string());
    let len = std::fs::metadata(path).map_err(io)?.len();
    if encoded_size(total.saturating_add(len)) > MAX_ATTACH_HARD {
        return Err(AttachWhy::TooBig);
    }
    // The file may grow after the size check; never read past the limit.
    let limit = (MAX_ATTACH_HARD * 3 / 4).saturating_sub(total);
    let mut data = Vec::with_capacity(len as usize);
    std::fs::File::open(path)
        .map_err(io)?
        .take(limit + 1)
        .read_to_end(&mut data)
        .map_err(io)?;
    if data.len() as u64 > limit {
        return Err(AttachWhy::TooBig);
    }
    Ok(data)
}

/// Seconds for the "Sending in N s" line.
fn count_secs(secs: u64) -> i32 {
    i32::try_from(secs).unwrap_or(i32::MAX)
}

// ------------------------------------------------------------ helpers

/// To and Cc for a reply to `m`. A reply to one's own message (a Sent item)
/// goes to its original To. Addresses compare by bare address, so excluding
/// `al@x.org` leaves `val@x.org` alone.
fn reply_recipients(
    m: &db::Message,
    reply_to: &str,
    me: &str,
    all: bool,
) -> (Vec<String>, Vec<String>) {
    let me = bare_address(me);
    let not_me = |a: &String| me.is_empty() || bare_address(a) != me;
    let mut to = if !me.is_empty() && bare_address(&m.from_addr) == me {
        mail::smtp::split_addresses(&m.to)
    } else if reply_to.is_empty() {
        vec![m.from_addr.clone()]
    } else {
        vec![reply_to.to_string()]
    };
    let mut cc = Vec::new();
    if all {
        let listed =
            |list: &[String], a: &String| list.iter().any(|t| bare_address(t) == bare_address(a));
        for a in mail::smtp::split_addresses(&m.to) {
            if not_me(&a) && !listed(&to, &a) {
                to.push(a);
            }
        }
        for a in mail::smtp::split_addresses(&m.cc) {
            if not_me(&a) && !listed(&to, &a) && !listed(&cc, &a) {
                cc.push(a);
            }
        }
    }
    (to, cc)
}

/// Where the caret most likely is after an edit turned `old` into `new`:
/// just past the changed span. A line field reports its text, not its
/// caret; within a run of equal characters this picks the run's end.
fn caret_after_edit(old: &str, new: &str) -> usize {
    let prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let room = old.len().min(new.len()) - prefix;
    let mut suffix = 0;
    for (a, b) in old.chars().rev().zip(new.chars().rev()) {
        if a != b || suffix + b.len_utf8() > room {
            break;
        }
        suffix += b.len_utf8();
    }
    new.len() - suffix
}

/// The address around byte `cursor` of an address field, as byte offsets
/// (start, cursor, end), with `cursor` moved onto the text. Addresses end
/// at a `,` or `;` outside quotes and angle brackets, as in
/// `smtp::split_addresses`.
fn address_at(text: &str, cursor: usize) -> (usize, usize, usize) {
    let mut cursor = cursor.min(text.len());
    while !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let (mut start, mut end) = (0, text.len());
    let (mut quoted, mut angle, mut escaped) = (false, false, false);
    for (i, c) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '<' if !quoted => angle = true,
            '>' if !quoted => angle = false,
            ',' | ';' if !quoted && !angle => {
                if i < cursor {
                    start = i + 1;
                } else {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }
    (start, cursor, end)
}

/// What is typed of the address at `cursor`, up to the caret.
fn typed_address(text: &str, cursor: usize) -> &str {
    let (start, cursor, _) = address_at(text, cursor);
    text[start..cursor]
        .trim()
        .trim_start_matches(['"', '<'])
        .trim_start()
}

/// Replace the address at `cursor` with `mailbox` and a `, ` separator
/// (taking the place of the one after it, if any). Returns the new text
/// and the caret, just past the separator.
fn complete_address(text: &str, cursor: usize, mailbox: &str) -> (String, usize) {
    let (start, _, end) = address_at(text, cursor);
    let head = text[..start].trim_end();
    let tail = &text[end..];
    let tail = tail.strip_prefix([',', ';']).unwrap_or(tail).trim_start();
    let mut out = String::with_capacity(text.len() + mailbox.len() + 3);
    out.push_str(head);
    if !head.is_empty() {
        out.push(' ');
    }
    out.push_str(mailbox);
    out.push_str(", ");
    let caret = out.len();
    out.push_str(tail);
    (out, caret)
}

/// `Name <addr>` or `addr` to lowercase `addr`.
fn bare_address(a: &str) -> String {
    let a = a.trim();
    let addr = match (a.rfind('<'), a.rfind('>')) {
        (Some(l), Some(r)) if l < r => &a[l + 1..r],
        _ => a,
    };
    addr.trim().to_lowercase()
}

fn prefixed(prefix: &str, subject: &str) -> String {
    let s = subject.trim();
    if s.get(..prefix.len())
        .is_some_and(|p| p.eq_ignore_ascii_case(prefix))
    {
        s.to_string()
    } else {
        format!("{prefix} {s}")
    }
}

/// The block a signature goes into the body as: two blank lines, the
/// RFC 3676 delimiter `-- ` (with its trailing space) and the signature.
/// Empty for no signature.
/// A done formatting edit showing the selection `anchor..cursor`.
fn body_edit(anchor: usize, cursor: usize) -> BodyEdit {
    let offset = |n: usize| i32::try_from(n).unwrap_or(i32::MAX);
    BodyEdit {
        done: true,
        anchor: offset(anchor),
        cursor: offset(cursor),
    }
}

pub(super) fn signature_block(signature: &str) -> String {
    let sig = signature.replace("\r\n", "\n");
    let sig = sig.trim_end();
    // A delimiter typed into the signature itself is not doubled.
    let sig = sig
        .strip_prefix("-- \n")
        .or_else(|| sig.strip_prefix("--\n"))
        .unwrap_or(sig);
    if sig.trim().is_empty() {
        String::new()
    } else {
        format!("\n\n-- \n{sig}")
    }
}

/// `body` with signature block `old` swapped for `new` (either may be
/// empty). `None` leaves the body alone: the block was edited, or with no
/// signature before, text was typed where the new one would go (above the
/// quote of a reply or forward).
fn swap_signature(body: &str, kind: Kind, old: &str, new: &str) -> Option<String> {
    if old == new {
        return None;
    }
    if !old.is_empty() {
        let i = body.find(old)?;
        return Some(format!("{}{new}{}", &body[..i], &body[i + old.len()..]));
    }
    match kind {
        Kind::New => Some(format!("{}{new}", body.trim_end())),
        _ if body.is_empty() || body.starts_with("\n\n") => Some(format!("{new}{body}")),
        _ => None,
    }
}

/// The line a reply's quote starts with, in the language shown.
fn attribution(m: &db::Message, heads: &Heads) -> String {
    (heads.wrote)(
        &format::full_date(heads.lang, m.date),
        &format::sender(&m.from_name, &m.from_addr),
    )
}

/// The quote below a reply or forward of `m` (`original`: its sanitized
/// HTML), and the line it starts with in the text quote. `None` for a new
/// message.
fn quote_block(
    kind: Kind,
    m: &db::Message,
    original: &str,
    heads: &Heads,
) -> Option<(String, String)> {
    use mail::compose_html;
    Some(match kind {
        Kind::New => return None,
        Kind::Reply | Kind::ReplyAll => {
            let line = attribution(m, heads);
            let block = compose_html::reply_quote(&line, original);
            (line, block)
        }
        Kind::Forward => {
            let from = format!("{} <{}>", m.from_name, m.from_addr);
            let date = format::full_date(heads.lang, m.date);
            let fields = [
                (heads.from.as_str(), from.trim()),
                (heads.date.as_str(), date.as_str()),
                (heads.to.as_str(), m.to.as_str()),
                (heads.subject.as_str(), m.subject.as_str()),
            ];
            let block = compose_html::forward_quote(&heads.forwarded, &fields, original);
            (heads.forwarded.clone(), block)
        }
    })
}

/// Text as HTML that shows it as it is: escaped, each line break kept.
fn text_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            out.push_str("<br>\n");
        }
        for c in line.chars() {
            match c {
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '&' => out.push_str("&amp;"),
                '"' => out.push_str("&quot;"),
                c => out.push(c),
            }
        }
    }
    out
}

/// The sanitized HTML body of raw message `raw`, for a reply or forward to
/// quote (`None`: a text message, or nothing safe to quote).
fn quotable(raw: Option<&[u8]>) -> Option<String> {
    mail::compose_html::sanitize(&mime::html_body(raw?)?)
}

/// A signature longer than this many lines is taken for something else.
const MAX_SIGNATURE_LINES: usize = 10;

/// `text` without its trailing signature (the last `-- ` line, RFC 3676,
/// and what follows, if that is at most a few lines), for quoting.
fn strip_signature(text: &str) -> &str {
    let mut cut = None;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "-- " {
            cut = Some(at);
        }
        at += line.len();
    }
    match cut {
        Some(i) if text[i..].lines().count() <= MAX_SIGNATURE_LINES + 1 => text[..i].trim_end(),
        _ => text,
    }
}

fn quote(text: &str) -> String {
    text.lines()
        .map(|l| {
            if l.starts_with('>') {
                format!(">{l}")
            } else {
                format!("> {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{MarkRead, NewHeader};
    use crate::mail::errors::Say;
    use slint::Rgb8Pixel;
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{Key, Platform, WindowAdapter, WindowEvent};

    mod rich_tests;
    mod tabs_tests;

    fn message(from: &str, to: &str, cc: &str) -> db::Message {
        db::Message {
            id: 1,
            account_id: 1,
            folder_id: 1,
            uid: 1,
            msgid: String::new(),
            subject: String::new(),
            from_name: String::new(),
            from_addr: from.into(),
            to: to.into(),
            cc: cc.into(),
            date: 0,
            seen: true,
            flagged: false,
            importance: db::Importance::Normal,
            raw: None,
        }
    }

    #[test]
    fn drafts_per_kind() {
        let new = Draft::new(7);
        assert_eq!((new.account_id, new.kind), (7, Kind::New));
        assert!(new.is_empty() && !new.keep());
        assert_eq!(new.send_block(), Some(SendBlock::NoRecipient));

        let mut m = message("bob@x.org", "al@x.org, val@x.org", "cy@x.org");
        m.id = 42;
        m.account_id = 3;
        m.subject = "Lunch".into();
        m.msgid = "m1@x.org".into();
        let body = Body {
            text: "Noon?\n> earlier".into(),
            references: "<m0@x.org> <m1@x.org>".into(),
            ..Default::default()
        };
        let reply = Draft::reply(&m, &body, "al@x.org", false, &Heads::en());
        assert_eq!(reply.kind, Kind::Reply);
        assert_eq!(reply.account_id, 3);
        assert_eq!((reply.to.as_str(), reply.cc.as_str()), ("bob@x.org", ""));
        assert!(!reply.show_cc);
        assert_eq!(reply.subject, "Re: Lunch");
        assert_eq!(reply.reply_to, Some(42));
        assert_eq!(reply.in_reply_to, "m1@x.org");
        assert_eq!(reply.references, body.references);
        assert!(reply.body.starts_with("\n\nOn "), "{}", reply.body);
        assert!(
            reply.body.ends_with("> Noon?\n>> earlier"),
            "{}",
            reply.body
        );
        // Filled in by the app, not by the user: nothing worth keeping.
        assert!(!reply.keep());

        // Reply all reuses reply_recipients: me left out, Cc shown.
        let all = Draft::reply(&m, &body, "al@x.org", true, &Heads::en());
        assert_eq!(all.kind, Kind::ReplyAll);
        assert_eq!(all.to, "bob@x.org, val@x.org");
        assert_eq!(all.cc, "cy@x.org");
        assert!(all.show_cc);
        let (to, cc) = reply_recipients(&m, "", "al@x.org", true);
        assert_eq!((all.to, all.cc), (to.join(", "), cc.join(", ")));

        let file = OutAttachment {
            filename: "a.pdf".into(),
            data: vec![1, 2],
        };
        let fwd = Draft::forward(&m, &body, vec![file.into()], &Heads::en());
        assert_eq!(fwd.kind, Kind::Forward);
        assert_eq!(fwd.subject, "Fwd: Lunch");
        assert!(fwd.to.is_empty() && fwd.in_reply_to.is_empty() && fwd.reply_to.is_none());
        assert_eq!(fwd.attachments.len(), 1);
        assert!(
            fwd.body
                .contains("---------- Forwarded message ----------\nFrom:  <bob@x.org>")
        );
        assert!(fwd.body.ends_with("Noon?\n> earlier"));
    }

    #[test]
    fn addresses_are_checked_on_leaving_and_rechecked_while_wrong() {
        let mut d = Draft::new(1);
        d.edit(ComposeField::To, "João Silva <j@x.pt>");
        // Typing alone shows nothing; leaving the field checks it.
        d.validate(ComposeField::To);
        assert_eq!(d.errors, [None, None, None]);
        assert_eq!(d.send_block(), None);

        d.edit(ComposeField::Cc, "ana@x.pt, j@");
        assert!(d.errors[1].is_none());
        d.validate(ComposeField::Cc);
        // The address at fault; the error words it in the language shown.
        assert_eq!(d.errors[1].as_deref(), Some("j@"));
        assert_eq!(
            d.send_block(),
            Some(SendBlock::FixAddress(ComposeField::Cc))
        );
        // Once shown, the error follows the typing and clears when right.
        d.edit(ComposeField::Cc, "ana@x.pt, j@x");
        assert_eq!(d.errors[1], None);
        assert_eq!(d.send_block(), None);

        assert!(address_error("João Silva j@x.pt").is_some());
        assert!(address_error("\"Silva, João\" <j@x.pt>").is_none());
        // Subject and body are never checked.
        d.edit(ComposeField::Subject, "<<<");
        d.validate(ComposeField::Subject);
        assert_eq!(d.send_block(), None);
    }

    #[test]
    fn attachment_mentions_in_english_and_portuguese() {
        for text in [
            "See attached.",
            "I attach the invoice",
            "The attachment is the signed copy",
            "Attachments: two",
            "Segue em anexo o relatório.",
            "Envio o documento anexado",
            "ANEXO: contrato",
            "(attached)",
        ] {
            assert!(mentions_attachment(text), "{text}");
        }
        for text in [
            "",
            "Nothing to see here",
            "The detached house",
            "reattach later",
            // Only the user's own text counts: not quoted lines...
            "Thanks!\n\nOn Monday, Bob wrote:\n> see attached\n>> anexo",
            // ...nor a forwarded message.
            "FYI\n\n---------- Forwarded message ----------\nSee attached.",
            // ...nor the signature.
            "Hi\n\n-- \nAna\nAnexos: ver site",
        ] {
            assert!(!mentions_attachment(text), "{text}");
        }
    }

    #[test]
    fn send_guards_ask_about_subject_and_attachments() {
        let mut d = Draft::new(1);
        d.edit(ComposeField::To, "ana@x.org");
        assert_eq!(send_guard(&d), Some(Guard::NoSubject));
        d.edit(ComposeField::Body, "The file is attached.");
        assert_eq!(send_guard(&d), Some(Guard::Both));
        d.edit(ComposeField::Subject, "  Report ");
        assert_eq!(send_guard(&d), Some(Guard::NoAttachment));
        d.attachments.push(Attached::from(OutAttachment {
            filename: "report.pdf".into(),
            data: vec![1],
        }));
        assert_eq!(send_guard(&d), None);
        // A forward with nothing attached is the user's choice to make.
        d.attachments.clear();
        d.edit(ComposeField::Body, "FYI");
        assert_eq!(send_guard(&d), None);
    }

    #[test]
    fn an_untouched_draft_is_replaced_and_a_typed_one_kept() {
        let mut drafts = Drafts::default();
        let first = drafts.open(Draft::new(1));
        let second = drafts.open(Draft::new(1));
        assert_eq!(drafts.len(), 1, "an empty draft is not kept");
        assert!(drafts.get(first).is_none());
        drafts
            .active_mut()
            .unwrap()
            .edit(ComposeField::Body, "hello");
        let third = drafts.open(Draft::new(1));
        assert_eq!(drafts.len(), 2);
        assert_eq!(drafts.newest_kept(), Some(second));
        drafts.remove(third);
        assert_eq!(drafts.active_id(), None);
        assert_eq!(drafts.newest_kept(), Some(second));
        // Typing then clearing everything leaves nothing to keep.
        let d = drafts.get_mut(second).unwrap();
        d.edit(ComposeField::Body, " ");
        assert!(!d.keep());
    }

    #[test]
    fn quotes_fold_out_of_the_composer_and_are_still_sent() {
        let m = message("Bob <bob@x.org>", "al@x.org", "");
        let body = Body {
            text: "Noon?\n> earlier".into(),
            ..Default::default()
        };
        // A reply: what is typed stays, the attribution and quote fold.
        let mut r = Draft::reply(&m, &body, "al@x.org", false, &Heads::en());
        r.sign("\n\n-- \nAl");
        let full = r.full_body();
        r.fold_quote();
        assert_eq!(r.body, "\n\n-- \nAl");
        assert!(r.quote.as_deref().unwrap().starts_with("\n\nOn "));
        assert_eq!(r.full_body(), full);
        r.body.insert_str(0, "Ok");
        assert_eq!(r.to_row(0).body, format!("Ok{full}"));
        // What is sent (outgoing's text part) is the full body.
        let sent = mail::compose_html::alternative(&r.full_body(), r.quoted_html.as_deref()).0;
        assert!(
            sent.starts_with("Ok\n") && sent.ends_with(">> earlier"),
            "{sent}"
        );

        // The body replaced as a whole (Ctrl+A, then typing) keeps its
        // distance from the quote, and reopening folds the same way.
        r.body = "Thanks.".into();
        let typed = r.full_body();
        assert!(typed.starts_with("Thanks.\n\nOn "), "{typed}");
        let mut again = Draft::new(1);
        again.kind = Kind::Reply;
        again.body = typed.clone();
        again.fold_quote();
        assert_eq!(again.body, "Thanks.");
        assert_eq!(again.full_body(), typed);
        assert_eq!(above_quote(&typed), "Thanks.");
        // A quote folded without blank lines above it still gets one.
        assert_eq!(
            with_quote("Hi", "On Mon, Bob wrote:\n> a"),
            "Hi\n\nOn Mon, Bob wrote:\n> a"
        );
        assert_eq!(
            with_quote("", "On Mon, Bob wrote:\n> a"),
            "On Mon, Bob wrote:\n> a"
        );

        // A forward, with its heading in any language.
        let pt = Heads {
            forwarded: "---------- Mensagem reencaminhada ----------".into(),
            from: "De".into(),
            date: "Data".into(),
            subject: "Assunto".into(),
            to: "Para".into(),
            ..Heads::en()
        };
        let mut f = Draft::forward(&m, &body, Vec::new(), &pt);
        f.fold_quote();
        assert_eq!(f.body, "");
        let q = f.quote.clone().unwrap();
        assert!(
            q.starts_with("\n\n---------- Mensagem reencaminhada ----------\nDe: "),
            "{q}"
        );
        // The words "anexo" in a forwarded message are not the user's.
        assert!(!mentions_attachment(
            "Hi\n---------- Mensagem reencaminhada ----------\nvai em anexo"
        ));

        // Unfolding gives the same text back; nothing to fold: no change.
        assert!(f.unfold_quote());
        assert!(!f.unfold_quote());
        let mut n = Draft::new(1);
        n.body = "Hi\n> not a quote of ours".into();
        n.fold_quote();
        assert!(n.quote.is_none());
        // Text above a reply's quote that ends in its own quoted lines.
        assert_eq!(
            quote_start("Hi\n\nOn Mon, Bob wrote:\n> a\n>\n> b\n"),
            Some(2)
        );
        assert_eq!(quote_start("Hi\nOn Mon, Bob wrote:\n> a"), Some(2));
        assert_eq!(quote_start("> only quotes\n> here"), None);
        assert_eq!(quote_start("Hi\n\n-- \n> sig"), None);
    }

    #[test]
    fn the_preview_draws_what_is_sent_within_its_caps() {
        let html = |body: &str| mail::compose_html::preview_html(body, None);
        let (short, tall, wide) = std::thread::Builder::new()
            .stack_size(PREVIEW_STACK)
            .spawn(move || {
                (
                    preview_page(html("Hi **Ana**"), 600.0, 1.0, false),
                    preview_page(html(&"A line.\n\n".repeat(2000)), 600.0, 1.0, false),
                    preview_page(html("Wide"), 1600.0, 1.5, false),
                )
            })
            .unwrap()
            .join()
            .unwrap();
        let short = short.unwrap();
        assert!(short.height > 0.0 && !short.cut);
        assert_eq!((short.pixels.width(), short.width), (600, 600.0));
        assert!(
            short.pixels.as_bytes().iter().any(|b| *b != 0),
            "something drawn"
        );
        // One band at most: the rest is said to be cut.
        let tall = tall.unwrap();
        assert!(tall.cut);
        assert_eq!(tall.pixels.height(), PREVIEW_MAX_ROWS);
        // Never laid out wider than 960 logical px (then drawn unstretched).
        let wide = wide.unwrap();
        assert_eq!(wide.width, 960.0);
        assert_eq!(wide.pixels.width(), 1440);
    }

    #[test]
    fn the_undo_send_offer_shortens_the_subject() {
        let _ui = crate::tr::tests::window();
        assert_eq!(short_subject("  Plans "), "Plans");
        assert_eq!(short_subject(""), "(no subject)");
        let long = "A subject that goes on and on well past the forty characters";
        let cut = short_subject(long);
        assert_eq!(cut, "A subject that goes on and on well past\u{2026}");
        assert_eq!(cut.chars().count(), OFFER_SUBJECT_CHARS);
        assert_eq!(
            short_subject(&"ç".repeat(50)).chars().count(),
            OFFER_SUBJECT_CHARS + 1
        );
    }

    #[test]
    fn subject_prefixes() {
        assert_eq!(prefixed("Re:", "Hello"), "Re: Hello");
        assert_eq!(prefixed("Re:", "RE: Hello"), "RE: Hello");
        assert_eq!(prefixed("Fwd:", "  "), "Fwd: ");
    }

    #[test]
    fn quoting() {
        assert_eq!(quote("a\n> b"), "> a\n>> b");
    }

    #[test]
    fn signature_blocks() {
        assert_eq!(signature_block(""), "");
        assert_eq!(signature_block(" \r\n "), "");
        assert_eq!(signature_block("Ana\r\nACME  \n\n"), "\n\n-- \nAna\nACME");
        // A typed delimiter is not doubled.
        assert_eq!(signature_block("-- \nAna"), "\n\n-- \nAna");
        assert_eq!(signature_block("--\nAna"), "\n\n-- \nAna");
    }

    #[test]
    fn signature_goes_in_once_per_kind() {
        let block = signature_block("Ana");
        let mut new = Draft::new(1);
        new.sign(&block);
        assert_eq!(new.body, "\n\n-- \nAna");
        // Only the signature: nothing written yet.
        assert!(new.is_empty() && !new.keep());

        let m = message("Bob <bob@x.org>", "ana@x.org", "");
        let body = Body {
            text: "See you at noon.\n-- \nBob\nBob's Bakery".into(),
            ..Default::default()
        };
        let mut reply = Draft::reply(&m, &body, "ana@x.org", false, &Heads::en());
        reply.sign(&block);
        // Above the quote, and the original's signature is not quoted.
        assert!(
            reply.body.starts_with("\n\n-- \nAna\n\nOn "),
            "{}",
            reply.body
        );
        assert!(
            reply.body.ends_with("wrote:\n> See you at noon."),
            "{}",
            reply.body
        );
        assert_eq!(reply.body.matches("-- ").count(), 1);

        let mut fwd = Draft::forward(&m, &body, Vec::new(), &Heads::en());
        fwd.sign(&block);
        assert!(
            fwd.body
                .starts_with(&format!("\n\n-- \nAna\n\n{FORWARDED}"))
        );
        // Forwarded as it was, signature included.
        assert!(fwd.body.ends_with("Bob's Bakery"));

        // A mailto: body comes first, then the signature.
        let mut mailto = Draft {
            body: "Hello".into(),
            ..Draft::new(1)
        };
        mailto.sign(&block);
        assert_eq!(mailto.body, "Hello\n\n-- \nAna");
        assert!(!mailto.is_empty());

        // No signature: nothing added.
        let mut plain = Draft::new(1);
        plain.sign("");
        assert_eq!(plain.body, "");
    }

    #[test]
    fn signature_swaps_only_while_untouched() {
        let (ana, work) = (signature_block("Ana"), signature_block("Ana\nACME"));
        let mut d = Draft::new(1);
        d.sign(&ana);
        d.edit(ComposeField::Body, &format!("Hi Bob{ana}"));
        assert!(d.resign(&work));
        assert_eq!(d.body, "Hi Bob\n\n-- \nAna\nACME");
        // Back, and to an account without one.
        assert!(d.resign(&ana));
        assert!(d.resign(""));
        assert_eq!(d.body, "Hi Bob");
        assert!(d.resign(&work));
        assert_eq!(d.body, "Hi Bob\n\n-- \nAna\nACME");

        // Edited signature: left alone from then on.
        d.edit(ComposeField::Body, "Hi Bob\n\n-- \nAna (mobile)\nACME");
        assert!(!d.resign(&ana));
        assert_eq!(d.body, "Hi Bob\n\n-- \nAna (mobile)\nACME");

        // A reply without a signature gets one above the quote, unless
        // something was typed there.
        let m = message("bob@x.org", "ana@x.org", "");
        let mut r = Draft::reply(&m, &Body::default(), "ana@x.org", false, &Heads::en());
        r.sign("");
        assert!(r.resign(&ana));
        assert!(r.body.starts_with("\n\n-- \nAna\n\nOn "), "{}", r.body);
        let mut r = Draft::reply(&m, &Body::default(), "ana@x.org", false, &Heads::en());
        r.sign("");
        r.body.insert_str(0, "Thanks!");
        assert!(!r.resign(&ana));

        // A saved draft whose signature isn't known is never touched.
        let mut saved = Draft::new(1);
        saved.body = "Hi\n\n-- \nOld".into();
        assert!(!saved.resign(&ana));
    }

    #[test]
    fn html_quotes_follow_the_text_quote() {
        use mail::compose_html::alternative;
        let mut m = message("bob@x.org", "al@x.org", "");
        m.from_name = "Bob".into();
        m.subject = "Lunch".into();
        let body = Body {
            text: "Noon?".into(),
            ..Default::default()
        };
        // As the app does it: the quote, then the signature above it.
        let mut reply = Draft::reply(&m, &body, "al@x.org", false, &Heads::en());
        reply.quote_html(&m, "<p>Noon?</p>", &Heads::en());
        reply.sign(&signature_block("Ana"));
        let typed = format!("Yes{}", reply.body);
        reply.edit(ComposeField::Body, &typed);
        let (text, html) = alternative(&reply.body, reply.quoted_html.as_deref());
        assert_eq!(text, typed);
        let html = html.unwrap();
        assert!(
            html.contains(
                "<p>Yes</p><div class=\"sig\">-- <br>Ana</div><div class=\"komorebi_quote\" style=\"margin-top:1em\">On "
            ),
            "{html}"
        );
        assert!(html.contains("<p>Noon?</p></blockquote>"), "{html}");

        let mut fwd = Draft::forward(&m, &body, Vec::new(), &Heads::en());
        fwd.quote_html(&m, "<p>Noon?</p>", &Heads::en());
        let html = alternative(&fwd.body, fwd.quoted_html.as_deref())
            .1
            .unwrap();
        assert!(
            html.contains("<b>From:</b></td><td>Bob &lt;bob@x.org&gt;</td>"),
            "{html}"
        );
        assert!(
            html.contains("<b>Subject:</b></td><td>Lunch</td>"),
            "{html}"
        );
        assert!(html.contains("</table><br><p>Noon?</p></div>"), "{html}");
        // The heading deleted: the text goes as typed.
        let cut = fwd.body.replace(FORWARDED, "");
        let html = alternative(&cut, fwd.quoted_html.as_deref()).1.unwrap();
        assert!(!html.contains("komorebi_quote"), "{html}");

        let mut new = Draft::new(1);
        new.quote_html(&m, "<p>Noon?</p>", &Heads::en());
        assert_eq!(new.quoted_html, None);
    }

    #[test]
    fn only_a_short_trailing_signature_is_stripped() {
        assert_eq!(strip_signature("Hi\n\n-- \nBob\n"), "Hi");
        assert_eq!(strip_signature("Hi\r\n-- \r\nBob\r\n"), "Hi");
        // The last delimiter counts.
        assert_eq!(strip_signature("A\n-- \nx\nB\n-- \nBob"), "A\n-- \nx\nB");
        // "--" without the space, or "-- " inside a line, is text.
        assert_eq!(strip_signature("Hi\n--\nBob"), "Hi\n--\nBob");
        assert_eq!(strip_signature("a -- b"), "a -- b");
        // A long tail (a quoted thread under a signature) stays.
        let long = format!("Hi\n-- \nBob\n{}", "> old\n".repeat(20));
        assert_eq!(strip_signature(&long), long);
    }

    #[test]
    fn attachment_limit_is_checked_before_reading() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let len = std::fs::metadata(&path).unwrap().len();
        let budget = MAX_ATTACH_HARD * 3 / 4;
        assert_eq!(read_attachment(&path, 0).unwrap().len() as u64, len);
        assert_eq!(
            read_attachment(&path, budget - len).unwrap().len() as u64,
            len
        );
        let err = read_attachment(&path, budget - len + 1).unwrap_err();
        assert_eq!(err, AttachWhy::TooBig);
        assert_eq!(encoded_size(3), 4);
    }

    #[test]
    fn dropped_paths_attach_files_only() {
        let dir = std::env::temp_dir().join(format!("komorebi-drop-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        std::fs::write(dir.join("Relatório.txt"), "olá").unwrap();
        let paths = [
            dir.join("Relatório.txt"),
            dir.join("folder"),
            dir.join("missing.pdf"),
        ];
        let (added, errors) = read_files(&paths, 0);
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].file.filename, "Relatório.txt");
        assert_eq!(added[0].file.data, "olá".as_bytes());
        assert_eq!(errors.len(), 2);
        assert_eq!(
            errors[0],
            AttachFail {
                name: "folder".into(),
                why: AttachWhy::Folder
            }
        );
        assert_eq!(errors[1].name, "missing.pdf");
        assert!(matches!(errors[1].why, AttachWhy::Io(_)), "{:?}", errors[1]);
        // Counted against what is attached already.
        let (added, errors) = read_files(&paths[..1], MAX_ATTACH_HARD * 3 / 4);
        assert!(added.is_empty());
        assert_eq!(errors[0].why, AttachWhy::TooBig);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn attachment_counter_thresholds() {
        let limit = MAX_ATTACHMENTS_BYTES;
        assert_eq!(counter_level(0), 0);
        assert_eq!(counter_level(limit * 4 / 5 - 1), 0);
        assert_eq!(counter_level(limit * 4 / 5), 1);
        assert_eq!(counter_level(limit), 1);
        assert_eq!(counter_level(limit + 1), 2);
        assert_eq!(counter_level(u64::MAX), 2);

        // Over the limit, Send is off.
        let mut d = Draft {
            to: "ana@example.org".into(),
            ..Default::default()
        };
        let file = |len: u64| {
            Attached::from(OutAttachment {
                filename: "a.bin".into(),
                data: vec![0; len as usize],
            })
        };
        d.attachments.push(file(limit * 3 / 4));
        assert_eq!(d.send_block(), None);
        d.attachments.push(file(3));
        assert_eq!(d.send_block(), Some(SendBlock::AttachmentsOver));
    }

    #[test]
    fn the_caret_follows_the_edit() {
        assert_eq!(caret_after_edit("", "a"), 1);
        assert_eq!(caret_after_edit("ana, b", "ana, bo"), 7);
        // Typed in the middle of a list.
        assert_eq!(caret_after_edit("a@x, b@x", "ajo@x, b@x"), 3);
        assert_eq!(caret_after_edit("joão, b", "joãoz, b"), 6);
        assert_eq!(caret_after_edit("abc", "ac"), 1);
        assert_eq!(caret_after_edit("ana", "x"), 1);
        assert_eq!(caret_after_edit("ana", ""), 0);
    }

    #[test]
    fn suggestions_complete_the_address_at_the_caret() {
        // The last address of a list.
        let text = "ana@x.pt, jo";
        assert_eq!(typed_address(text, text.len()), "jo");
        let (out, caret) = complete_address(text, text.len(), "João <joao@x.pt>");
        assert_eq!(out, "ana@x.pt, João <joao@x.pt>, ");
        assert_eq!(caret, out.len());
        // The caret in the middle of a list: the separator after the
        // address is reused and the rest stays.
        let text = "ana@x.pt; jo, bob@x.pt";
        let cursor = text.find(',').unwrap();
        assert_eq!(typed_address(text, cursor), "jo");
        let (out, caret) = complete_address(text, cursor, "João <joao@x.pt>");
        assert_eq!(out, "ana@x.pt; João <joao@x.pt>, bob@x.pt");
        assert_eq!(&out[caret..], "bob@x.pt");
        // Only what is typed before the caret is matched.
        assert_eq!(typed_address("johnny", 2), "jo");
        assert_eq!(complete_address("johnny", 2, "J <j@x>").0, "J <j@x>, ");
        // A quoted name with a comma is one address.
        let text = r#""Doe, Jane" <jane@x.pt>, "Sil"#;
        assert_eq!(typed_address(text, text.len()), "Sil");
        let quoted = mail::smtp::mailbox("Silva, Ana", "ana@x.pt");
        let (out, _) = complete_address(text, text.len(), &quoted);
        assert_eq!(out, r#""Doe, Jane" <jane@x.pt>, "Silva, Ana" <ana@x.pt>, "#);
        assert_eq!(mail::smtp::split_addresses(&out).len(), 2);
        assert_eq!(typed_address(r#""Doe, Ja"#, 8), "Doe, Ja");
        // Any caret, even one off the text or inside a character.
        for cursor in 0..12 {
            let _ = complete_address("ã, é", cursor, "x@y");
            let _ = typed_address("ã, é", cursor);
        }
        assert_eq!(complete_address("", 0, "x@y"), ("x@y, ".to_string(), 5));
    }

    #[test]
    fn reply_recipients_cases() {
        let m = message(
            "bob@x.org",
            "Al <AL@x.org>, val@x.org, bob@x.org",
            "al@x.org, cy@x.org",
        );
        let me = "al@x.org";
        assert_eq!(
            reply_recipients(&m, "", me, false),
            (vec!["bob@x.org".into()], vec![])
        );
        // val@ is not mistaken for al@, and the sender is not repeated.
        assert_eq!(
            reply_recipients(&m, "", me, true),
            (
                vec!["bob@x.org".into(), "val@x.org".into()],
                vec!["cy@x.org".into()]
            )
        );
        assert_eq!(
            reply_recipients(&m, "list@x.org", me, false).0,
            ["list@x.org"]
        );
        // Replying to a Sent item goes to its original recipients.
        let sent = message("Al@x.org", "Bob <bob@x.org>, val@x.org", "cy@x.org");
        assert_eq!(
            reply_recipients(&sent, "", me, false),
            (vec!["Bob <bob@x.org>".into(), "val@x.org".into()], vec![])
        );
        assert_eq!(
            reply_recipients(&sent, "", me, true),
            (
                vec!["Bob <bob@x.org>".into(), "val@x.org".into()],
                vec!["cy@x.org".into()]
            )
        );
        assert_eq!(bare_address("\"a <b>\" <C@x.org> "), "c@x.org");
    }

    // ------------------------------------------------ the real app, headless

    /// The first window gets the harness's own adapter; message windows
    /// (`app::windows`) get one each.
    struct TestPlatform(Rc<MinimalSoftwareWindow>, Cell<bool>);

    impl Platform for TestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            if self.1.replace(true) {
                return Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer));
            }
            Ok(self.0.clone())
        }
    }

    /// The app on a headless window over a temp cache with one account and
    /// one message. The account is added after start-up, so no worker (and
    /// no network) ever runs.
    struct Harness {
        /// Hidden and dropped before the cache file is removed (the shown
        /// window keeps the app, and so the cache, open).
        ui: Option<AppWindow>,
        window: Rc<MinimalSoftwareWindow>,
        path: PathBuf,
        message: i64,
    }

    impl Harness {
        fn new(name: &str) -> Self {
            let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
            slint::platform::set_platform(Box::new(TestPlatform(window.clone(), Cell::new(false))))
                .unwrap();
            let ui = AppWindow::new().unwrap();
            ui.window().set_size(slint::PhysicalSize::new(1280, 800));
            ui.show().unwrap();
            let path = std::env::temp_dir().join(format!(
                "komorebi-composer-{}-{name}.sqlite3",
                std::process::id()
            ));
            remove_db(&path);
            // English, whatever language Windows is set to; new messages in
            // plain text (the formatted editor's tests ask for it).
            let first = Db::open(&path).unwrap();
            first.set_setting("language", "en").unwrap();
            first.set_setting("compose_format", "plain").unwrap();
            drop(first);
            install(&ui, path.clone(), &|_| {}).unwrap();
            let mut db = Db::open(&path).unwrap();
            let server = |host: &str| Server {
                host: host.into(),
                port: 993,
                security: Security::Tls,
            };
            let account = db
                .add_account(&NewAccount {
                    imap_user: String::new(),
                    smtp_user: String::new(),
                    email: "me@example.org".into(),
                    name: "Me".into(),
                    imap: server("imap.example.invalid"),
                    smtp: server("smtp.example.invalid"),
                    save_sent: true,
                })
                .unwrap();
            let folder = db
                .upsert_folder(account, "INBOX", "Inbox", 0, Role::Inbox)
                .unwrap();
            let header = NewHeader {
                uid: 1,
                msgid: "m1@example.org".into(),
                subject: "Lunch".into(),
                from_name: "Bob".into(),
                from_addr: "bob@example.org".into(),
                to: "me@example.org".into(),
                date: 1_700_000_000,
                ..Default::default()
            };
            db.insert_headers(account, folder, &[header]).unwrap();
            let message = db.message_ids(&[folder], "").unwrap()[0];
            let raw = b"From: Bob <bob@example.org>\r\nTo: me@example.org\r\n\
                Subject: Lunch\r\nMessage-ID: <m1@example.org>\r\n\r\nNoon?\r\n";
            db.set_body(message, raw, "Noon?", false, Default::default())
                .unwrap();
            with_app(|a| {
                // No tip or What's new bar by itself: a slow test would
                // see one appear (their tests start them by hand).
                a.tips_auto = false;
                a.load_accounts().unwrap();
                a.reload_rows();
                // Opening marks read at once here; the delay has its own test.
                a.mark_read = MarkRead::Now;
                // Never the real credential store.
                a.forget_password = Rc::new(|_| Ok(()));
            })
            .unwrap();
            assert!(with_app(|a| a.workers.is_empty()).unwrap());
            let h = Harness {
                ui: Some(ui),
                window,
                path,
                message,
            };
            h.frame();
            h
        }

        /// Run change handlers and timers and draw a frame.
        fn frame(&self) {
            for _ in 0..3 {
                slint::platform::update_timers_and_animations();
                self.window.request_redraw();
                self.window.draw_if_needed(|r| {
                    let size = self.window.size();
                    let (w, h) = (size.width as usize, size.height as usize);
                    let mut buf = vec![Rgb8Pixel::default(); w * h];
                    r.render(&mut buf, w);
                });
            }
        }

        /// Press `text` with `mods` held, then release everything.
        fn keys(&self, mods: &[Key], text: &str) {
            let send = |e| self.ui().window().dispatch_event(e);
            for m in mods {
                send(WindowEvent::KeyPressed { text: (*m).into() });
            }
            send(WindowEvent::KeyPressed { text: text.into() });
            send(WindowEvent::KeyReleased { text: text.into() });
            for m in mods.iter().rev() {
                send(WindowEvent::KeyReleased { text: (*m).into() });
            }
            self.frame();
        }

        fn app<R>(&self, f: impl FnOnce(&mut App) -> R) -> R {
            let r = with_app(f).expect("app installed");
            self.frame();
            r
        }

        fn ui(&self) -> &AppWindow {
            self.ui.as_ref().expect("window alive")
        }

        fn status(&self) -> String {
            self.ui().get_status().into()
        }

        /// The Undo offer at the bottom of the list.
        fn undo(&self) -> String {
            self.ui().get_undo_text().into()
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            APP.with(|cell| cell.borrow_mut().take());
            if let Some(ui) = self.ui.take() {
                ui.hide().ok();
            }
            remove_db(&self.path);
        }
    }

    fn remove_db(path: &Path) {
        for ext in ["", "-wal", "-shm"] {
            let mut p = path.as_os_str().to_owned();
            p.push(ext);
            std::fs::remove_file(p).ok();
        }
        std::fs::remove_dir_all(db::drafts_dir(path)).ok();
    }

    /// The row menu (built as it opens) has an entry titled `title`
    /// (before its accelerator text).
    fn menu_has(ui: &AppWindow, title: &str) -> bool {
        [
            ui.get_row_menu_1(),
            ui.get_row_menu_2(),
            ui.get_row_menu_3(),
            ui.get_row_menu_4(),
            ui.get_row_menu_5(),
            ui.get_row_menu_6(),
        ]
        .iter()
        .flat_map(|m| m.iter().collect::<Vec<_>>())
        .any(|e| e.title.split('\t').next() == Some(title))
    }

    /// Type into a field the way the window reports it.
    fn type_into(ui: &AppWindow, field: ComposeField, text: &str) {
        match field {
            ComposeField::To => ui.set_c_to(text.into()),
            ComposeField::Cc => ui.set_c_cc(text.into()),
            ComposeField::Bcc => ui.set_c_bcc(text.into()),
            ComposeField::Subject => ui.set_c_subject(text.into()),
            ComposeField::Body => ui.set_c_body(text.into()),
        }
        ui.invoke_c_edited(field, text.into());
    }

    /// The formatted draft's text as its input proxy holds it (empty: the
    /// draft shown is plain, or none is).
    fn rich_text(h: &Harness) -> String {
        h.app(|a| {
            a.drafts
                .active()
                .and_then(|d| d.rich.as_ref())
                .map(|e| e.text().to_string())
                .unwrap_or_default()
        })
    }

    #[test]
    fn added_files_update_the_counter_and_are_saved() {
        let h = Harness::new("attach");
        // Nothing dropped: nothing happens.
        h.app(|a| a.attach_dropped(Vec::new()));
        assert!(!h.ui().get_composing());
        // A drop with no composer open starts a new message for the files
        // (the overlay said "Drop to attach to a new message"; reading them
        // runs on a thread this test has no loop for).
        h.app(|a| a.attach_dropped(vec![std::env::temp_dir()]));
        assert!(h.ui().get_composing());
        assert_eq!(h.ui().get_c_title(), "New message");
        type_into(h.ui(), ComposeField::To, "ana@example.org");
        h.ui().set_file_hover(true);
        let id = h.app(|a| a.drafts.active_id().unwrap());
        let file = |name: &str, len: u64| {
            Attached::from(OutAttachment {
                filename: name.into(),
                data: vec![1; len as usize],
            })
        };
        // 85% of the counter: amber, Send stays on.
        let raw = MAX_ATTACHMENTS_BYTES * 85 / 100 * 3 / 4;
        h.app(|a| a.add_attachments(id, vec![file("a.pdf", raw)], Vec::new()));
        assert_eq!(h.ui().get_c_attachments().row_count(), 1);
        assert_eq!(h.ui().get_c_attach_total(), "Attachments 21.2 MB of 25 MB");
        assert_eq!(h.ui().get_c_attach_level(), 1);
        assert_eq!(h.ui().get_c_send_reason(), "");
        h.app(|a| {
            let d = a.drafts.get(id).unwrap();
            assert!(d.touched && d.row.is_some() && d.attachments[0].saved.is_some());
        });

        // Past 25 MB: red, Send off; past the hard limit: refused.
        let over = MAX_ATTACH_HARD * 3 / 4 - raw - 2;
        h.app(|a| {
            a.add_attachments(
                id,
                vec![
                    file("b.bin", MAX_ATTACHMENTS_BYTES / 4),
                    file("c.bin", over),
                ],
                vec![AttachFail {
                    name: "x.txt".into(),
                    why: AttachWhy::Io("gone".into()),
                }],
            )
        });
        assert_eq!(h.ui().get_c_attachments().row_count(), 2);
        assert_eq!(h.ui().get_c_attach_level(), 2);
        assert_eq!(
            h.ui().get_c_send_reason(),
            "Attachments are over 25 MB; remove some to send."
        );
        assert_eq!(
            h.status(),
            "x.txt: gone; c.bin: over Komorebi's 100 MB attachment limit"
        );
        h.ui().invoke_remove_attachment(1);
        h.frame();
        assert_eq!(h.ui().get_c_attach_level(), 1);
        assert_eq!(h.ui().get_c_send_reason(), "");

        // A drop ends the highlight.
        h.app(|a| a.attach_dropped(Vec::new()));
        assert!(!h.ui().get_file_hover());
    }

    #[test]
    fn message_actions_wait_while_composing() {
        let h = Harness::new("guard");
        let id = h.message;
        h.app(|a| a.select_message(id));
        h.ui().invoke_compose();
        h.frame();
        assert!(h.ui().get_composing());
        type_into(h.ui(), ComposeField::Body, "Draft text");

        h.ui().invoke_reply();
        h.ui().invoke_reply_all();
        h.ui().invoke_forward();
        h.ui().invoke_toggle_read();
        h.ui().invoke_toggle_flag();
        h.ui().invoke_archive();
        h.ui().invoke_delete();
        h.ui().invoke_select_offset(1);
        h.frame();

        assert!(h.ui().get_composing());
        assert_eq!(h.ui().get_c_title(), "New message");
        h.app(|a| {
            assert_eq!(a.drafts.len(), 1);
            assert_eq!(a.drafts.active().unwrap().body, "Draft text");
            assert_eq!(a.open.as_ref().map(|o| o.msg.id), Some(id));
            let m = a.db.message(id).unwrap().expect("not deleted");
            assert!(m.seen && !m.flagged);
        });

        // Back in the reader the same actions work again, and the draft
        // in progress is kept.
        h.ui().invoke_c_close();
        h.frame();
        assert!(!h.ui().get_composing());
        assert_eq!(h.status(), "Draft saved");
        h.ui().invoke_reply();
        h.frame();
        assert!(h.ui().get_composing());
        assert_eq!(h.ui().get_c_title(), "Reply");
        assert_eq!(h.app(|a| a.drafts.len()), 2);
    }

    #[test]
    fn a_second_compose_opens_a_tab_and_switching_round_trips() {
        let h = Harness::new("switch");
        h.ui().invoke_compose();
        h.frame();
        assert_eq!(h.ui().get_c_focus(), ComposeField::To);
        h.ui().invoke_c_reveal(ComposeField::Cc);
        let typed = [
            (ComposeField::To, "Ana <ana@example.org>"),
            (ComposeField::Cc, "cy@example.org"),
            (ComposeField::Bcc, "dee@example.org"),
            (ComposeField::Subject, "Plans"),
            (ComposeField::Body, "Line one\nLine two"),
        ];
        for (field, text) in typed {
            type_into(h.ui(), field, text);
        }
        h.app(|a| {
            a.drafts.active_mut().unwrap().attachments.push(
                OutAttachment {
                    filename: "notes.txt".into(),
                    data: b"x".to_vec(),
                }
                .into(),
            );
            a.show_compose_attachments();
        });

        // Selecting a message moves the draft to a tab of its own (in the
        // background); a reply then takes the reading pane.
        let id = h.message;
        h.app(|a| a.select_message(id));
        assert!(!h.ui().get_composing());
        assert_eq!(h.status(), "Draft kept in a tab");
        assert_eq!(h.app(|a| a.tabs.len()), 2);
        assert_eq!(h.app(|a| a.tabs.active()), 0);
        h.ui().invoke_reply();
        h.frame();
        assert_eq!(h.ui().get_c_title(), "Reply");
        assert_eq!(h.ui().get_c_to(), "bob@example.org");
        assert_eq!(h.ui().get_c_subject(), "Re: Lunch");
        assert_eq!(h.ui().get_c_cc(), "");
        assert!(!h.ui().get_c_show_cc());
        assert_eq!(h.ui().get_c_attachments().row_count(), 0);
        assert_eq!(h.ui().get_c_focus(), ComposeField::Body);
        assert_eq!(h.app(|a| a.drafts.len()), 2);

        // Esc on the untouched reply drops it. New mail is always blank
        // (and dropped again untouched); the first draft's tab comes back
        // from Drafts, every field as typed.
        h.ui().invoke_c_close();
        h.frame();
        assert_eq!(h.app(|a| a.drafts.len()), 1);
        h.ui().invoke_compose();
        h.frame();
        assert_eq!(h.ui().get_c_to(), "");
        assert_eq!(h.ui().get_c_body(), "");
        h.ui().invoke_c_close();
        h.frame();
        h.app(|a| a.set_view(View::Drafts));
        let row = h.ui().get_rows().row_data(0).unwrap();
        h.ui().invoke_message_selected(row.id);
        h.frame();
        assert!(h.ui().get_composing());
        assert_eq!(h.app(|a| a.tabs.active()), 1, "its tab");
        assert_eq!(h.ui().get_c_title(), "New message");
        assert_eq!(h.ui().get_c_to(), typed[0].1);
        assert_eq!(h.ui().get_c_cc(), typed[1].1);
        assert_eq!(h.ui().get_c_bcc(), typed[2].1);
        assert_eq!(h.ui().get_c_subject(), typed[3].1);
        assert_eq!(h.ui().get_c_body(), typed[4].1);
        assert!(h.ui().get_c_show_cc());
        assert_eq!(h.ui().get_c_attachments().row_count(), 1);
        assert_eq!(h.ui().get_c_send_reason(), "");

        // New mail while it shows keeps it and starts another, in a tab
        // to its right; switching back finds every field as it was.
        h.ui().invoke_compose();
        h.frame();
        assert_eq!(h.ui().get_c_to(), "");
        assert_eq!(h.app(|a| a.drafts.len()), 2);
        assert_eq!(h.app(|a| (a.tabs.len(), a.tabs.active())), (3, 2));
        h.ui().invoke_tab_activate(1);
        h.frame();
        assert_eq!(h.ui().get_c_to(), typed[0].1);
        assert_eq!(h.ui().get_c_body(), typed[4].1);
        assert_eq!(h.ui().get_c_attachments().row_count(), 1);
    }

    #[test]
    fn send_is_always_on_and_the_field_says_why() {
        let h = Harness::new("send-on");
        h.ui().invoke_compose();
        h.frame();
        type_into(h.ui(), ComposeField::Subject, "Plans");
        // No recipient: Send still goes through to the check, which puts
        // the sentence under To and the focus in it; nothing is sent.
        h.ui().invoke_send();
        h.frame();
        assert!(h.ui().get_composing());
        assert_eq!(h.ui().get_c_to_error(), "Add a recipient to send.");
        assert_eq!(h.ui().get_c_focus(), ComposeField::To);
        assert_eq!(h.status(), "");
        assert!(h.app(|a| a.db.outbox_ids("").unwrap().is_empty()));
        // Typing an address clears it at once.
        type_into(h.ui(), ComposeField::To, "ana@example.org");
        assert_eq!(h.ui().get_c_to_error(), "");
        assert_eq!(h.ui().get_c_send_reason(), "");
    }

    #[test]
    fn composer_keys_and_inline_validation() {
        let h = Harness::new("keys");
        h.ui().invoke_compose();
        h.frame();
        // New message: To has the focus.
        h.keys(&[], "a");
        assert_eq!(h.ui().get_c_to(), "a");
        assert_eq!(h.app(|a| a.drafts.active().unwrap().to.clone()), "a");
        assert_eq!(h.ui().get_c_to_error(), "");

        // Alt+S sends (refused here: To says why, inline, and keeps the
        // focus; no status line) and types nothing.
        h.keys(&[Key::Alt], "s");
        assert_eq!(h.ui().get_c_to(), "a");
        assert_eq!(h.status(), "");
        assert_eq!(h.ui().get_c_focus(), ComposeField::To);
        assert_eq!(h.ui().get_c_to_error(), "“a” is not a valid address");
        assert_eq!(h.ui().get_c_send_reason(), "Fix the To address to send.");

        // Fixing it clears the error as it is typed.
        h.keys(&[], "@x.pt");
        assert_eq!(h.ui().get_c_to(), "a@x.pt");
        assert_eq!(h.ui().get_c_to_error(), "");
        assert_eq!(h.ui().get_c_send_reason(), "");

        // Ctrl+Shift+B shows Cc and Bcc and moves to Bcc.
        h.keys(&[Key::Control, Key::Shift], "B");
        assert!(h.ui().get_c_show_cc());
        h.keys(&[], "b@");
        assert_eq!(h.ui().get_c_bcc(), "b@");
        assert_eq!(h.ui().get_c_cc(), "");
        // Leaving Bcc for Cc checks Bcc.
        h.keys(&[Key::Control, Key::Shift], "C");
        assert_eq!(h.ui().get_c_bcc_error(), "“b@” is not a valid address");
        assert_eq!(h.ui().get_c_send_reason(), "Fix the Bcc address to send.");

        // Ctrl+Enter in a line field sends (refused: the Bcc error, which
        // takes the focus).
        h.keys(&[Key::Control], "\n");
        assert_eq!(h.ui().get_c_focus(), ComposeField::Bcc);
        assert_eq!(h.status(), "");
        assert!(h.ui().get_composing());

        // Esc hides the composer and keeps the draft.
        h.keys(&[], "\u{1b}");
        assert!(!h.ui().get_composing());
        assert_eq!(h.status(), "Draft saved");
        assert_eq!(h.app(|a| a.drafts.active().unwrap().bcc.clone()), "b@");
    }

    #[test]
    fn recipient_suggestions_from_the_address_index() {
        let h = Harness::new("suggest");
        h.ui().invoke_compose();
        h.frame();
        // Bob, the sender of the cached message, is in the index.
        h.keys(&[], "b");
        assert_eq!(h.ui().get_c_suggestions().row_count(), 0);
        h.keys(&[], "o");
        let shown = h.ui().get_c_suggestions();
        assert_eq!(shown.row_count(), 1);
        assert_eq!(shown.row_data(0).unwrap().addr, "bob@example.org");
        let field = |h: &Harness| h.app(|a| a.drafts.suggest.as_ref().map(|s| s.field));
        assert_eq!(field(&h), Some(ComposeField::To));
        // Esc closes the suggestions, not the composer.
        h.keys(&[], "\u{1b}");
        assert!(h.ui().get_composing());
        assert_eq!(h.ui().get_c_suggestions().row_count(), 0);
        // Typing brings them back; Down wraps round; Enter takes one and
        // the caret goes after it.
        h.keys(&[], "b");
        assert_eq!(h.ui().get_c_suggestions().row_count(), 1);
        let down = SharedString::from(Key::DownArrow);
        h.keys(&[], &down);
        assert_eq!(h.ui().get_c_suggest_index(), 0);
        h.keys(&[], "\n");
        assert_eq!(h.ui().get_c_to(), "Bob <bob@example.org>, ");
        assert_eq!(h.ui().get_c_suggestions().row_count(), 0);
        assert!(h.ui().get_composing());
        h.keys(&[], "x");
        assert_eq!(h.ui().get_c_to(), "Bob <bob@example.org>, x");
        assert_eq!(
            h.app(|a| a.drafts.active().unwrap().to.clone()),
            "Bob <bob@example.org>, x"
        );
        // An address the draft already has is not suggested again.
        h.keys(&[], "\u{8}");
        h.keys(&[], "bo");
        assert_eq!(h.ui().get_c_suggestions().row_count(), 0);
        // Tab takes one in Cc; leaving the field hides them.
        type_into(h.ui(), ComposeField::To, "");
        h.keys(&[Key::Control, Key::Shift], "C");
        h.keys(&[], "bob");
        assert_eq!(field(&h), Some(ComposeField::Cc));
        h.keys(&[], "\t");
        assert_eq!(h.ui().get_c_cc(), "Bob <bob@example.org>, ");
        h.keys(&[], "bo");
        assert_eq!(h.ui().get_c_suggestions().row_count(), 0);
        type_into(h.ui(), ComposeField::Cc, "");
        h.keys(&[], "bo");
        assert_eq!(h.ui().get_c_suggestions().row_count(), 1);
        h.keys(&[Key::Control, Key::Shift], "B");
        assert_eq!(h.ui().get_c_suggestions().row_count(), 0);
        // Without suggestions Esc still hides the composer.
        h.keys(&[], "\u{1b}");
        assert!(!h.ui().get_composing());
    }

    #[test]
    fn reply_focuses_the_top_of_the_body() {
        let h = Harness::new("reply");
        let id = h.message;
        h.app(|a| a.select_message(id));
        h.ui().invoke_reply();
        h.frame();
        h.keys(&[], "O");
        h.keys(&[], "k");
        // The quote is folded behind "Show quoted text": the body shows
        // what is typed, and what is saved and sent still has the quote.
        let body: String = h.ui().get_c_body().into();
        assert_eq!(body, "Ok");
        assert!(h.ui().get_c_quote_folded());
        let full = h.app(|a| a.drafts.active().unwrap().full_body());
        assert!(full.starts_with("Ok\n\nOn "), "{full}");
        assert!(full.ends_with("> Noon?"), "{full}");
        assert_eq!(h.app(|a| a.drafts.active().unwrap().body.clone()), body);
        // Esc saves the whole of it; reopening folds the quote again.
        h.keys(&[], "\u{1b}");
        let db = Db::open(&h.path).unwrap();
        let row = db.draft_ids("").unwrap()[0];
        assert_eq!(db.draft(row).unwrap().unwrap().body, full);
        h.ui().invoke_reply();
        h.frame();
        assert_eq!(h.ui().get_c_body(), "Ok");
        assert!(h.ui().get_c_quote_folded());
        // Show quoted text puts it back as it was (nothing to save).
        h.ui().invoke_c_unfold_quote();
        h.frame();
        assert_eq!(h.ui().get_c_body(), full.as_str());
        assert!(!h.ui().get_c_quote_folded());
        assert!(!h.app(|a| a.drafts.active().unwrap().dirty));
    }

    /// The formatting toolbar's spike, kept as its guard: the body's
    /// selection offsets (internal TextInput properties) are UTF-8 byte
    /// offsets after typing multi-byte text, selecting with the keyboard
    /// and undo, and a restored selection is exactly the one computed.
    #[test]
    fn formatting_keys_act_on_the_body_selection() {
        // A formatted reply; plain-text messages have no formatting keys.
        let h = Harness::new("format");
        h.app(|a| a.compose_rich = true);
        let id = h.message;
        h.app(|a| a.select_message(id));
        h.ui().invoke_reply();
        h.frame();
        let para = || {
            h.app(|a| {
                let ed = a.drafts.active().unwrap().rich.as_ref().unwrap();
                match &ed.doc().blocks[0] {
                    model::Block::Para(p) => p.clone(),
                    other => panic!("{other:?}"),
                }
            })
        };
        let first_line = || para().plain_text();
        let styled = |text: &str| {
            para()
                .runs
                .iter()
                .find(|r| r.text.contains(text))
                .map(|r| r.style.clone())
                .unwrap_or_default()
        };
        let key = |k: Key| SharedString::from(k);
        let (ctrl, shift) = (Key::Control, Key::Shift);
        // Typed one character at a time; a dead key or an IME commits its
        // composed text the same way.
        for c in ["a", "ç", "ã", "o", " ", "😀"] {
            h.keys(&[], c);
        }
        assert_eq!(first_line(), "ação 😀");

        // The emoji, selected with Shift+Left, goes bold and back.
        h.keys(&[shift], &key(Key::LeftArrow));
        h.keys(&[ctrl], "b");
        assert!(styled("😀").b && !styled("ação").b);
        h.keys(&[ctrl], "b");
        assert!(!styled("😀").b);
        // The selection stays: typing replaces it.
        h.keys(&[], "x");
        assert_eq!(first_line(), "ação x");
        // Ctrl+Z undoes the typing, then each formatting edit, in order.
        h.keys(&[ctrl], "z");
        assert_eq!(first_line(), "ação 😀");
        h.keys(&[ctrl], "z");
        assert!(styled("😀").b);
        h.keys(&[ctrl], "z");
        assert!(!styled("😀").b);

        // The whole line: italic, then a list item (Shift+8 on a US and a
        // Portuguese layout), then numbered (Shift+7 on both).
        h.keys(&[], &key(Key::End));
        h.keys(&[shift], &key(Key::Home));
        h.keys(&[ctrl], "i");
        assert!(styled("ação").i && styled("😀").i);
        h.keys(&[ctrl, shift], "*");
        assert_eq!(para().list, Some((model::ListKind::Bullet, 0)));
        h.keys(&[ctrl, shift], "(");
        assert_eq!(para().list, None);
        h.keys(&[ctrl, shift], "/");
        assert_eq!(para().list, Some((model::ListKind::Number, 0)));
        h.keys(&[ctrl, shift], "7");
        assert_eq!(para().list, None);

        // Ctrl+K asks for the address; one that can't be followed is
        // refused, and Esc there closes only the field.
        h.keys(&[], &key(Key::End));
        h.keys(&[], " ");
        h.keys(&[ctrl], "k");
        h.keys(&[], "javascript:x");
        h.keys(&[], "\n");
        assert_eq!(first_line(), "ação 😀 ");
        h.keys(&[], "\u{1b}");
        assert!(h.ui().get_composing());
        h.keys(&[], "see");
        assert_eq!(first_line(), "ação 😀 see");
        h.keys(&[shift, ctrl], &key(Key::LeftArrow));
        h.keys(&[ctrl], "k");
        h.keys(&[], "example.org");
        h.keys(&[], "\n");
        assert_eq!(styled("see").link.as_deref(), Some("https://example.org/"));
        // The caret is after the link, back in the body; what is typed
        // there is not part of it.
        h.keys(&[], ".");
        assert_eq!(first_line(), "ação 😀 see.");
        assert_eq!(styled(".").link, None);

        // Ctrl+Shift+Q quotes the paragraph (a letter: no layout moves
        // it), and again takes it off.
        h.keys(&[ctrl, shift], "Q");
        assert_eq!(para().quote, 1);
        h.keys(&[ctrl, shift], "Q");
        assert_eq!(para().quote, 0);

        // Ctrl+Shift+B is still Bcc; the stored quote is still sent.
        h.keys(&[ctrl, shift], "B");
        assert!(h.ui().get_c_show_cc());
        let body = h.app(|a| a.drafts.active().unwrap().full_body());
        assert!(body.contains("Noon?"), "{body}");
    }

    /// The reader says the importance its sender set; a body cached before
    /// v13 (its row still normal) tells it as the message opens, and the
    /// row keeps it.
    #[test]
    fn the_reader_shows_the_senders_importance() {
        let h = Harness::new("reader-importance");
        let id = h.message;
        let mut db = Db::open(&h.path).unwrap();
        let raw = b"From: Bob <bob@example.org>\r\nTo: me@example.org\r\nSubject: Lunch\r\n\
            Importance: high\r\nContent-Type: text/plain\r\n\r\nNoon?\r\n";
        // As cached before v13: the body is here, the row says normal.
        db.set_body(id, raw, "Noon?", false, Default::default())
            .unwrap();
        assert_eq!(
            db.header(id).unwrap().unwrap().importance,
            db::Importance::Normal
        );
        assert!(!h.ui().get_rows().row_data(0).unwrap().high_importance);
        h.app(|a| a.select_message(id));
        h.frame();
        assert_eq!(h.ui().get_r_importance(), 1);
        assert_eq!(
            db.header(id).unwrap().unwrap().importance,
            db::Importance::High
        );
        let row = h.ui().get_rows().row_data(0).unwrap();
        assert!(row.high_importance && !row.low_importance);

        // Low, as synced: the reader's quiet note and the row's mark.
        let m = db.message(id).unwrap().unwrap();
        db.insert_headers(
            m.account_id,
            m.folder_id,
            &[db::NewHeader {
                uid: m.uid + 1,
                subject: "Later".into(),
                date: m.date + 60,
                importance: db::Importance::Low,
                ..Default::default()
            }],
        )
        .unwrap();
        h.app(|a| a.reload_rows());
        let low = h.ui().get_rows().row_data(0).unwrap();
        assert!(
            low.low_importance && !low.high_importance,
            "{}",
            low.subject
        );
        h.app(|a| a.select_message(i64::from(low.id)));
        h.frame();
        assert_eq!(h.ui().get_r_importance(), 2);
    }

    #[test]
    fn an_html_reply_quotes_the_original_in_its_html_part() {
        // A formatted reply (plain-text messages send no HTML).
        let h = Harness::new("html-reply");
        h.app(|a| a.compose_rich = true);
        let jobs = hold_builds(&h);
        let raw = b"From: Bob <bob@example.org>\r\nTo: me@example.org\r\nSubject: Lunch\r\n\
            Message-ID: <m1@example.org>\r\nMIME-Version: 1.0\r\n\
            Content-Type: text/html; charset=utf-8\r\n\r\n\
            <p style=\"color:#336\">Noon at <b>Rua Augusta</b>?</p>\
            <img src=\"https://t.example.invalid/p.gif\">\r\n";
        let mut db = Db::open(&h.path).unwrap();
        db.set_body(h.message, raw, "Noon", false, Default::default())
            .unwrap();
        let id = h.message;
        h.app(|a| a.select_message(id));
        h.ui().invoke_reply();
        h.frame();
        h.keys(&[], "O");
        h.keys(&[], "k");
        let quoted = h.app(|a| {
            let ed = a.drafts.active().unwrap().rich.as_ref().unwrap();
            ed.doc()
                .blocks
                .iter()
                .find_map(|b| match b {
                    model::Block::Quote(q) => Some(q.html().to_string()),
                    _ => None,
                })
                .expect("an HTML quote")
        });
        assert!(quoted.contains("<b>Rua Augusta</b>"), "{quoted}");
        assert!(!quoted.contains("p.gif"), "{quoted}");

        h.ui().invoke_send();
        h.frame();
        let (row, _) = outbox_row(&h).expect("sent");
        // Saved with the draft, so Undo and a restart keep it.
        let saved = db.draft(row).unwrap().unwrap().body_rich.unwrap();
        assert!(saved.contains("<b>Rua Augusta</b>"), "{saved}");
        assert_eq!(finish_build(&h, &jobs), Ok(true));
        let seq = h.app(|a| a.send_seq);
        h.app(|a| a.send_due(row, seq));
        let sent = db.claim_outbox(row).unwrap().expect("queued").raw;

        // The text part has the quote as `> ` lines; the HTML part has the
        // typed text and the original's sanitized HTML instead.
        let text = mime::parse_body(&sent).text;
        assert!(text.starts_with("Ok\n"), "{text}");
        assert!(text.contains("> Noon at Rua Augusta?"), "{text}");
        let html = mime::html_body(&sent).expect("an HTML part");
        assert!(
            html.contains("Ok") && html.contains("<b>Rua Augusta</b>"),
            "{html}"
        );
        assert!(
            !html.contains("&gt; Noon") && !html.contains("p.gif"),
            "{html}"
        );
    }
    // ------------------------------------------------ saved drafts

    /// A cache file for `store`/`load`, with one account; removed on drop.
    struct Cache {
        db: Db,
        account: i64,
        /// Declared after `db`, so the cache is closed before the files go.
        path: TempPath,
    }

    struct TempPath(PathBuf);

    impl std::ops::Deref for TempPath {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempPath {
        fn drop(&mut self) {
            remove_db(&self.0);
        }
    }

    impl Cache {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "komorebi-drafts-{}-{name}.sqlite3",
                std::process::id()
            ));
            remove_db(&path);
            let db = Db::open(&path).unwrap();
            let server = |host: &str| Server {
                host: host.into(),
                port: 993,
                security: Security::Tls,
            };
            let account = db
                .add_account(&NewAccount {
                    imap_user: String::new(),
                    smtp_user: String::new(),
                    email: "me@example.org".into(),
                    name: "Me".into(),
                    imap: server("imap.example.invalid"),
                    smtp: server("smtp.example.invalid"),
                    save_sent: true,
                })
                .unwrap();
            Cache {
                db,
                account,
                path: TempPath(path),
            }
        }
    }

    fn typed(account: i64) -> Draft {
        let mut d = Draft::new(account);
        d.edit(ComposeField::To, "Ana <ana@example.org>");
        d.edit(ComposeField::Subject, "Plans");
        d.edit(ComposeField::Body, "Line one\nLine two");
        d
    }

    #[test]
    fn empty_and_untouched_drafts_are_not_saved() {
        let c = Cache::new("empty");
        let mut d = Draft::new(c.account);
        assert_eq!(
            store(&c.db, &mut d, "me@example.org").unwrap(),
            Stored::Nothing
        );
        // Typed, then cleared again: still nothing.
        d.edit(ComposeField::Body, "x");
        d.edit(ComposeField::Body, " ");
        assert_eq!(
            store(&c.db, &mut d, "me@example.org").unwrap(),
            Stored::Nothing
        );
        // A reply only the app filled in is not saved either.
        let mut m = message("bob@x.org", "me@example.org", "");
        m.account_id = c.account;
        let mut reply = Draft::reply(&m, &Body::default(), "me@example.org", false, &Heads::en());
        assert!(!reply.is_empty());
        assert_eq!(
            store(&c.db, &mut reply, "me@example.org").unwrap(),
            Stored::Nothing
        );
        assert_eq!(c.db.count_drafts().unwrap(), 0);
        assert!(d.row.is_none() && d.message_id.is_empty());
    }

    #[test]
    fn importance_is_saved_and_sent() {
        use db::Importance;
        let c = Cache::new("importance");
        let mut d = typed(c.account);
        d.importance = Importance::High;
        assert_eq!(
            store(&c.db, &mut d, "me@example.org").unwrap(),
            Stored::Inserted
        );
        let row = d.row.expect("saved");
        let (back, _) = load(&Db::open(&c.path).unwrap(), row).unwrap().unwrap();
        assert_eq!(back.importance, Importance::High);
        // Sent with it.
        let account = c.db.accounts().unwrap().remove(0);
        let raw = mail::smtp::build_raw(outgoing(&c.db, &account, back)).unwrap();
        assert_eq!(mail::mime::parse_header(&raw).importance, Importance::High);
        // Back to normal: saved, and sent with neither header.
        d.importance = Importance::Normal;
        d.dirty = true;
        assert_eq!(
            store(&c.db, &mut d, "me@example.org").unwrap(),
            Stored::Updated
        );
        let (back, _) = load(&c.db, row).unwrap().unwrap();
        assert_eq!(back.importance, Importance::Normal);
        let raw = mail::smtp::build_raw(outgoing(&c.db, &account, back)).unwrap();
        let raw = String::from_utf8(raw).unwrap();
        assert!(!raw.contains("Importance:") && !raw.contains("X-Priority:"));
    }

    #[test]
    fn saved_drafts_round_trip_with_a_stable_message_id() {
        let c = Cache::new("round-trip");
        let mut d = typed(c.account);
        d.kind = Kind::ReplyAll;
        d.edit(ComposeField::Cc, "cy@example.org");
        d.edit(ComposeField::Bcc, "dee@example.org");
        d.reply_to = None;
        d.in_reply_to = "m1@x.org".into();
        d.references = "<m0@x.org> <m1@x.org>".into();
        d.quoted_html = Some("<!--komorebi-quote 1 0-->quote".into());
        d.attachments.push(
            OutAttachment {
                filename: "notes.txt".into(),
                data: b"hello".to_vec(),
            }
            .into(),
        );
        assert_eq!(
            store(&c.db, &mut d, "me@example.org").unwrap(),
            Stored::Inserted
        );
        let row = d.row.expect("saved");
        let id = d.message_id.clone();
        assert!(id.ends_with("@example.org>"), "{id}");
        assert!(!d.dirty);
        let file = d.attachments[0].saved.expect("copied");
        // Nothing changed: nothing written. An edit is one update; the
        // attachment is not copied again.
        assert_eq!(
            store(&c.db, &mut d, "me@example.org").unwrap(),
            Stored::Nothing
        );
        d.edit(ComposeField::Body, "Line one\nLine two\nLine three");
        assert_eq!(
            store(&c.db, &mut d, "me@example.org").unwrap(),
            Stored::Updated
        );
        assert_eq!(c.db.draft_files(row).unwrap().len(), 1);
        assert_eq!(d.attachments[0].saved, Some(file));

        // As after a restart: every field, the attachment and the same
        // Message-ID come back.
        let (back, missing) = load(&Db::open(&c.path).unwrap(), row).unwrap().unwrap();
        assert!(missing.is_empty());
        assert_eq!(back.row, Some(row));
        assert_eq!(back.kind, Kind::ReplyAll);
        assert_eq!(back.to, "Ana <ana@example.org>");
        assert_eq!(back.cc, "cy@example.org");
        assert_eq!(back.bcc, "dee@example.org");
        assert!(back.show_cc);
        assert_eq!(back.subject, "Plans");
        assert_eq!(back.body, "Line one\nLine two\nLine three");
        assert_eq!(back.in_reply_to, "m1@x.org");
        assert_eq!(back.references, "<m0@x.org> <m1@x.org>");
        assert_eq!(back.quoted_html, d.quoted_html);
        assert_eq!(back.message_id, id);
        assert_eq!(back.attachments.len(), 1);
        assert_eq!(back.attachments[0].file.filename, "notes.txt");
        assert_eq!(back.attachments[0].file.data, b"hello");
        assert_eq!(back.attachments[0].saved, Some(file));
        assert!(back.keep() && !back.dirty);
        let mut back = back;
        assert_eq!(
            store(&c.db, &mut back, "other@example.org").unwrap(),
            Stored::Nothing
        );
        assert_eq!(back.message_id, id);

        // A copy that vanished is left out and named.
        std::fs::remove_file(
            db::drafts_dir(&c.path)
                .join(row.to_string())
                .join(file.to_string()),
        )
        .unwrap();
        let (back, missing) = load(&c.db, row).unwrap().unwrap();
        assert!(back.attachments.is_empty());
        assert_eq!(missing, ["notes.txt"]);
    }

    #[test]
    fn emptying_a_saved_draft_deletes_it_and_its_copies() {
        let c = Cache::new("emptied");
        let mut d = typed(c.account);
        d.attachments.push(
            OutAttachment {
                filename: "a.bin".into(),
                data: vec![7; 10],
            }
            .into(),
        );
        store(&c.db, &mut d, "me@example.org").unwrap();
        let row = d.row.unwrap();
        let folder = db::drafts_dir(&c.path).join(row.to_string());
        assert!(folder.exists());
        for field in [ComposeField::To, ComposeField::Subject, ComposeField::Body] {
            d.edit(field, "");
        }
        d.attachments.clear();
        assert_eq!(
            store(&c.db, &mut d, "me@example.org").unwrap(),
            Stored::Deleted(None)
        );
        assert!(d.row.is_none());
        assert!(c.db.draft(row).unwrap().is_none());
        assert!(!folder.exists());
        assert!(load(&c.db, row).unwrap().is_none());
    }

    #[test]
    fn edits_within_two_seconds_end_in_one_save() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        // The timer starts with the first edit and comes due 2 s later.
        let edits = [0, 400, 1_500];
        let mut last = at(0);
        let mut due = at(2_000);
        let mut saves = Vec::new();
        for _ in 0..10 {
            for &e in &edits {
                if at(e) <= due {
                    last = last.max(at(e));
                }
            }
            match autosave_wait(last, due) {
                Some(rest) => due += rest,
                None => {
                    saves.push(due.duration_since(t0).as_millis());
                    break;
                }
            }
        }
        assert_eq!(saves, [3_500]);
        assert_eq!(autosave_wait(at(0), at(2_000)), None);
        assert_eq!(autosave_wait(at(0), at(5_000)), None);
        assert_eq!(
            autosave_wait(at(1_000), at(2_000)),
            Some(Duration::from_millis(1_000))
        );
    }

    #[test]
    fn server_copies_go_at_most_every_thirty_seconds() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        // An autosave every 2 s for 70 s of typing.
        let mut last = None;
        let mut copies = Vec::new();
        for s in (2..=70).step_by(2) {
            if server_save_due(last, at(s)) {
                last = Some(at(s));
                copies.push(s);
            }
        }
        assert_eq!(copies, [2, 32, 62]);
        assert!(server_save_due(None, t0));
        assert!(!server_save_due(Some(at(10)), at(39)));
        assert!(server_save_due(Some(at(10)), at(40)));
    }

    #[test]
    fn closing_saves_and_the_drafts_view_reopens_it() {
        let h = Harness::new("drafts-view");
        h.ui().invoke_compose();
        h.frame();
        type_into(h.ui(), ComposeField::To, "ana@example.org");
        type_into(h.ui(), ComposeField::Subject, "Plans");
        // Esc saves at once (no autosave wait).
        h.keys(&[], "\u{1b}");
        assert_eq!(h.status(), "Draft saved");
        let db = Db::open(&h.path).unwrap();
        let rows = db.draft_ids("").unwrap();
        assert_eq!(rows.len(), 1);
        let saved = db.draft(rows[0]).unwrap().unwrap();
        assert_eq!(
            (saved.to.as_str(), saved.subject.as_str()),
            ("ana@example.org", "Plans")
        );
        assert!(!saved.message_id.is_empty());

        // The nav lists Drafts with its count; the view lists the draft.
        let nav = |h: &Harness| -> Vec<(i32, String, i32)> {
            h.ui()
                .get_nav_items()
                .iter()
                .map(|n| (n.kind, n.label.to_string(), n.count))
                .collect()
        };
        assert!(nav(&h).contains(&(3, "Drafts".into(), 1)));
        h.ui().invoke_nav_clicked(NavItem {
            kind: 3,
            ..Default::default()
        });
        h.frame();
        assert_eq!(h.ui().get_list_title(), "Drafts");
        assert_eq!(h.ui().get_selected_folder(), -1);
        let list = h.ui().get_rows();
        assert_eq!(list.row_count(), 1);
        let row = list.row_data(0).unwrap();
        assert_eq!(row.sender, "To: ana@example.org");
        assert_eq!(row.subject, "Plans");
        assert_eq!(row.id as i64, rows[0]);
        // The arrows move to a draft without opening it (the composer
        // would take the keys); no "N selected" panel.
        h.ui().invoke_select_offset(1);
        h.frame();
        assert!(!h.ui().get_composing() && !h.ui().get_has_message());
        assert_eq!(h.ui().get_selected_index(), 0);
        assert_eq!(h.ui().get_selected_count(), 0);

        // After a restart (nothing in memory) a click opens it.
        h.app(|a| a.drafts = Drafts::default());
        h.ui().invoke_message_selected(row.id);
        h.frame();
        assert!(h.ui().get_composing());
        assert_eq!(h.ui().get_c_to(), "ana@example.org");
        assert_eq!(h.ui().get_c_subject(), "Plans");
        assert_eq!(h.ui().get_selected_id(), row.id);

        // An edit is saved by the autosave once typing pauses.
        type_into(h.ui(), ComposeField::Body, "See you");
        h.app(|a| {
            a.autosave();
            assert!(a.drafts.active().unwrap().dirty, "edited just now: waits");
            a.last_edit = Instant::now() - AUTOSAVE_AFTER;
            a.autosave();
            assert!(!a.drafts.active().unwrap().dirty);
        });
        assert_eq!(db.draft(rows[0]).unwrap().unwrap().body, "See you");

        // Discard asks first; Cancel keeps it, Discard deletes it.
        h.ui().invoke_discard();
        h.frame();
        assert!(h.ui().get_c_confirm_discard());
        assert!(h.ui().get_composing());
        h.keys(&[], "\u{1b}");
        assert!(!h.ui().get_c_confirm_discard());
        assert!(db.draft(rows[0]).unwrap().is_some());
        h.ui().invoke_discard();
        h.ui().invoke_discard_confirmed();
        h.frame();
        assert!(!h.ui().get_composing());
        assert!(db.draft(rows[0]).unwrap().is_none());
        assert_eq!(h.ui().get_rows().row_count(), 0);
        assert_eq!(
            h.ui().get_list_empty(),
            "No drafts. Select New mail to start one."
        );
        // The entry stays, empty, after its view closes too.
        assert!(nav(&h).contains(&(3, "Drafts".into(), 0)));
        let inbox = h.app(|a| a.folders[0].id);
        h.app(|a| a.set_view(View::Folder(inbox)));
        assert!(nav(&h).contains(&(3, "Drafts".into(), 0)));
    }

    #[test]
    fn drafts_answer_enter_delete_and_their_menu() {
        let h = Harness::new("drafts-keys");
        let db = Db::open(&h.path).unwrap();
        let account = h.app(|a| a.accounts[0].id);
        for subject in ["older", "newer"] {
            db.insert_draft(&db::DraftRow {
                account_id: account,
                to: "ana@example.org".into(),
                subject: subject.into(),
                body: "Hi".into(),
                message_id: format!("<{subject}@example.org>"),
                ..Default::default()
            })
            .unwrap();
        }
        h.app(|a| {
            a.count_drafts_and_outbox();
            a.set_view(View::Drafts);
        });
        assert_eq!(h.ui().get_rows().row_count(), 2);
        // Down, Down, then Enter: the second (older) draft opens.
        h.ui().invoke_select_offset(1);
        h.ui().invoke_select_offset(1);
        h.frame();
        assert_eq!(h.ui().get_selected_index(), 1);
        assert!(!h.ui().invoke_open_current());
        h.frame();
        assert!(h.ui().get_composing());
        assert_eq!(h.ui().get_c_subject(), "older");
        h.ui().invoke_c_close();
        h.frame();
        // Its row menu (Open, Discard) is on the row under the pointer.
        let first = h.ui().get_rows().row_data(0).unwrap().id;
        assert!(h.ui().invoke_message_context(first));
        assert_eq!(h.ui().get_selected_index(), 0);
        // Delete asks first, as Discard does; Cancel keeps the draft.
        h.ui().invoke_delete();
        h.frame();
        assert!(h.ui().get_c_confirm_discard());
        assert_eq!(h.ui().get_c_subject(), "newer");
        h.ui().set_c_confirm_discard(false);
        h.ui().invoke_c_close();
        h.frame();
        assert_eq!(db.count_drafts().unwrap(), 2);
        // New mail never brings a saved draft back.
        h.ui().invoke_compose();
        h.frame();
        assert_eq!(
            (h.ui().get_c_subject().as_str(), h.ui().get_c_to().as_str()),
            ("", "")
        );
    }

    #[test]
    fn an_untouched_draft_is_discarded_without_asking() {
        let h = Harness::new("discard-empty");
        let id = h.message;
        h.app(|a| a.select_message(id));
        h.ui().invoke_reply();
        h.frame();
        h.ui().invoke_discard();
        h.frame();
        assert!(!h.ui().get_c_confirm_discard());
        assert!(!h.ui().get_composing());
        assert_eq!(h.app(|a| a.drafts.len()), 0);
        assert_eq!(Db::open(&h.path).unwrap().count_drafts().unwrap(), 0);
    }

    /// High or Low importance (or another From account) is not writing:
    /// a composer with nothing typed is still dropped silently on Esc,
    /// never saved here or copied to the server's Drafts.
    #[test]
    fn importance_alone_does_not_keep_a_draft() {
        let h = Harness::new("importance-empty");
        h.ui().invoke_compose();
        h.frame();
        h.ui().invoke_c_set_importance(1);
        h.ui().invoke_c_set_importance(2);
        assert_eq!(h.ui().get_c_importance(), 2);
        assert_eq!(Db::open(&h.path).unwrap().count_drafts().unwrap(), 0);
        assert!(!h.app(|a| a.drafts.active().unwrap().keep()));
        h.ui().invoke_c_close();
        h.frame();
        assert!(!h.ui().get_composing());
        assert_ne!(h.status(), "Draft saved");
        assert_eq!(h.app(|a| a.drafts.len()), 0);
        assert_eq!(Db::open(&h.path).unwrap().count_drafts().unwrap(), 0);

        // A reply has recipients and a subject: High importance on it is a
        // choice, saved with it.
        let id = h.message;
        h.app(|a| a.select_message(id));
        h.ui().invoke_reply();
        h.frame();
        h.ui().invoke_c_set_importance(1);
        h.ui().invoke_c_close();
        h.frame();
        assert_eq!(h.status(), "Draft saved");
        let db = Db::open(&h.path).unwrap();
        assert_eq!(db.count_drafts().unwrap(), 1);
        let row = db.draft_for_message(id).unwrap().expect("saved");
        let (d, _) = load(&db, row).unwrap().unwrap();
        assert_eq!(d.importance, db::Importance::High);
    }

    #[test]
    fn reply_reopens_the_saved_draft_and_the_row_is_tagged() {
        let h = Harness::new("reply-draft");
        let id = h.message;
        let tagged = |h: &Harness| h.ui().get_rows().row_data(0).unwrap().draft;
        assert!(!tagged(&h));
        h.app(|a| a.select_message(id));
        h.ui().invoke_reply();
        h.frame();
        h.keys(&[], "O");
        h.keys(&[], "k");
        // Opening another message moves the reply to a tab of its own:
        // saved, and said; the message shows "You started a reply" with
        // Resume.
        h.app(|a| a.select_message(id));
        assert!(!h.ui().get_composing());
        assert!(tagged(&h));
        assert_eq!(h.status(), "Draft kept in a tab");
        assert!(h.ui().get_r_draft_reply());
        let db = Db::open(&h.path).unwrap();
        let row = db.draft_for_message(id).unwrap().expect("saved");
        assert_eq!(db.count_drafts().unwrap(), 1);

        // After a restart, Reply (or Reply all) brings that draft back
        // instead of starting another.
        h.app(|a| {
            a.drafts = Drafts::default();
            a.tabs = crate::app::tabs::Tabs::default();
            a.show_tabs();
        });
        h.ui().invoke_reply_all();
        h.frame();
        assert!(h.ui().get_composing());
        assert_eq!(h.ui().get_c_title(), "Reply");
        assert_eq!(h.ui().get_c_body(), "Ok");
        let body = h.app(|a| a.drafts.active().unwrap().full_body());
        assert!(body.starts_with("Ok\n\nOn "), "{body}");
        assert_eq!(h.app(|a| a.drafts.active().unwrap().row), Some(row));
        // The caption says when it was saved.
        assert!(
            h.ui().get_c_saved().starts_with("Draft saved "),
            "{}",
            h.ui().get_c_saved()
        );
        assert_eq!(db.count_drafts().unwrap(), 1);

        // Saved drafts are never offered on a status line at start: the
        // Drafts entry, always in the pane, holds them.
        h.app(|a| {
            a.set_status("", false);
            a.count_drafts_and_outbox();
        });
        assert_eq!(h.status(), "");
        assert_eq!(h.ui().get_status_action(), "");
    }

    /// Captured workers for the harness's account: what the UI queues for
    /// the server arrives on the receiver (nothing connects).
    fn capture_jobs(h: &Harness) -> std::sync::mpsc::Receiver<Job> {
        let (account, notify) = h.app(|a| (a.accounts[0].clone(), a.notify.clone()));
        let id = account.id;
        let (workers, jobs) = Workers::capture(account, h.path.clone(), notify);
        h.app(|a| a.workers.insert(id, workers));
        jobs
    }

    fn server_saves(jobs: &std::sync::mpsc::Receiver<Job>) -> Vec<i64> {
        jobs.try_iter()
            .filter_map(|j| match j {
                Job::SaveDraft { row } => Some(row),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn draft_rows_show_the_text_without_its_marks() {
        let h = Harness::new("draft-marks");
        h.ui().invoke_compose();
        h.frame();
        type_into(h.ui(), ComposeField::To, "ana@example.org");
        type_into(
            h.ui(),
            ComposeField::Body,
            "The **report** is _ready_, 2 * 3",
        );
        h.ui().invoke_c_close();
        h.frame();
        h.app(|a| a.set_view(View::Drafts));
        let row = h.ui().get_rows().row_data(0).unwrap();
        assert_eq!(row.preview, "The report is ready, 2 * 3");
    }

    /// A reply's body set as a whole (the snapshot action, or Ctrl+A and
    /// typing) is still a blank line away from the quote in what is saved,
    /// shows as typed when the draft reopens, and previews without the
    /// quote's heading.
    #[test]
    fn a_replaced_reply_body_keeps_its_distance_from_the_quote() {
        let h = Harness::new("reply-replaced");
        let id = h.message;
        h.app(|a| a.select_message(id));
        h.ui().invoke_reply();
        h.frame();
        h.ui()
            .invoke_c_edited(ComposeField::Body, "Thanks, see you then.".into());
        let full = h.app(|a| a.drafts.active().unwrap().full_body());
        assert!(full.starts_with("Thanks, see you then.\n\nOn "), "{full}");
        h.ui().invoke_c_close();
        h.frame();
        let db = Db::open(&h.path).unwrap();
        let row = db.draft_ids("").unwrap()[0];
        assert_eq!(db.draft(row).unwrap().unwrap().body, full);
        h.ui().invoke_reply();
        h.frame();
        assert_eq!(h.ui().get_c_body(), "Thanks, see you then.");
        assert!(h.ui().get_c_quote_folded());
        h.ui().invoke_c_close();
        h.frame();
        h.app(|a| a.set_view(View::Drafts));
        let row = h.ui().get_rows().row_data(0).unwrap();
        assert_eq!(row.preview, "Thanks, see you then.");
    }

    #[test]
    fn every_way_out_of_the_composer_saves_here_and_on_the_server() {
        let h = Harness::new("park-server");
        let jobs = capture_jobs(&h);
        let db = Db::open(&h.path).unwrap();

        // A click on a message moves the draft to a tab of its own: saved
        // here, its server copy queued at once, and that said.
        h.ui().invoke_compose();
        h.frame();
        type_into(h.ui(), ComposeField::To, "ana@example.org");
        type_into(h.ui(), ComposeField::Subject, "Plans");
        h.ui().invoke_message_selected(h.message as i32);
        h.frame();
        assert!(!h.ui().get_composing());
        assert_eq!(h.status(), "Draft kept in a tab");
        let row = db.draft_ids("").unwrap()[0];
        assert_eq!(db.draft(row).unwrap().unwrap().subject, "Plans");
        assert_eq!(server_saves(&jobs), [row]);

        // Close (as Esc; in its tab, the tab closes): the same, once more
        // after an edit.
        h.app(|a| a.set_status("", false));
        h.app(|a| a.open_saved(row));
        assert_eq!(h.app(|a| a.tabs.active()), 1, "its tab");
        assert!(h.ui().get_c_saved().starts_with("Draft saved "));
        type_into(h.ui(), ComposeField::Body, "See you");
        h.ui().invoke_c_close();
        h.frame();
        assert!(!h.ui().get_composing());
        assert_eq!(h.status(), "Draft saved");
        assert_eq!(db.draft(row).unwrap().unwrap().body, "See you");
        assert_eq!(server_saves(&jobs), [row]);

        // A click on an Outbox row: the same.
        let outbox = settled_row(&h, db::Outbox::Failed, "permanent error (550): no");
        h.app(|a| a.set_view(View::Outbox));
        h.app(|a| a.open_saved(row));
        type_into(h.ui(), ComposeField::Body, "See you Monday");
        h.ui().invoke_message_selected(outbox as i32);
        h.frame();
        assert!(!h.ui().get_composing() && h.ui().get_o_show());
        assert_eq!(db.draft(row).unwrap().unwrap().body, "See you Monday");
        assert_eq!(server_saves(&jobs), [row]);

        // An empty draft is dropped, not saved.
        h.ui().invoke_compose();
        h.frame();
        server_saves(&jobs);
        h.ui().invoke_c_close();
        h.frame();
        assert!(server_saves(&jobs).is_empty());
        assert_eq!(db.draft_ids("").unwrap(), [row]);
    }

    #[test]
    fn discarding_removes_the_draft_here_and_its_server_copy() {
        let h = Harness::new("discard-copy");
        let jobs = capture_jobs(&h);
        let db = Db::open(&h.path).unwrap();
        let account = h.app(|a| a.accounts[0].id);
        let drafts = db
            .upsert_folder(account, "Drafts", "Drafts", 0, Role::Drafts)
            .unwrap();
        h.ui().invoke_compose();
        h.frame();
        type_into(h.ui(), ComposeField::To, "ana@example.org");
        h.ui().invoke_c_close();
        h.frame();
        let row = db.draft_ids("").unwrap()[0];
        let copy = db::ServerCopy {
            account_id: account,
            folder_id: drafts,
            uidvalidity: 7,
            uid: 12,
        };
        db.record_server_copy(row, 0, Some(&copy)).unwrap();
        server_saves(&jobs);

        // What the prompt says is what happens: gone here, with its
        // attachments, and its copy in the server's Drafts removed.
        h.app(|a| a.open_saved(row));
        h.ui().invoke_discard();
        h.frame();
        assert!(h.ui().get_c_confirm_discard());
        let said = crate::tr::with(|t| t.invoke_q_discard_text().to_string()).unwrap();
        assert_eq!(
            said,
            "It is deleted here and from your Drafts folder on the server, with its attachments."
        );
        h.ui().invoke_discard_confirmed();
        h.frame();
        assert!(db.draft(row).unwrap().is_none());
        let removed: Vec<db::ServerCopy> = jobs
            .try_iter()
            .filter_map(|j| match j {
                Job::DeleteDraft(c) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(removed, [copy]);
    }

    #[test]
    fn closing_the_window_saves_the_draft() {
        let h = Harness::new("window-close");
        h.ui().invoke_compose();
        h.frame();
        type_into(h.ui(), ComposeField::To, "ana@example.org");
        type_into(h.ui(), ComposeField::Body, "Typed just now");
        // No Esc and no autosave yet: only the window closing.
        h.ui().window().dispatch_event(WindowEvent::CloseRequested);
        let db = Db::open(&h.path).unwrap();
        let rows = db.draft_ids("").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(db.draft(rows[0]).unwrap().unwrap().body, "Typed just now");
    }

    // ------------------------------------------------- outbox and undo send

    type Jobs = Rc<RefCell<Vec<BuildJob>>>;

    /// Builds are collected here instead of running on a thread, so a test
    /// decides when each one finishes.
    fn hold_builds(h: &Harness) -> Jobs {
        let jobs: Jobs = Rc::default();
        let sink = jobs.clone();
        h.app(|a| a.builder = Rc::new(move |job| sink.borrow_mut().push(job)));
        jobs
    }

    /// Finish the oldest held build the way the build thread does.
    fn finish_build(h: &Harness, jobs: &Jobs) -> Result<bool, String> {
        let job = jobs.borrow_mut().remove(0);
        let (row, seq) = (job.row, job.seq);
        let result = job.run();
        h.app(|a| a.on_built(row, seq, result.clone()));
        result
    }

    /// A new message, typed and ready to send.
    fn write_message(h: &Harness) {
        h.ui().invoke_compose();
        h.frame();
        h.ui().invoke_c_reveal(ComposeField::Cc);
        type_into(h.ui(), ComposeField::To, "Ana <ana@example.org>");
        type_into(h.ui(), ComposeField::Cc, "cy@example.org");
        type_into(h.ui(), ComposeField::Subject, "Plans");
        type_into(h.ui(), ComposeField::Body, "Report attached.\nSee you");
        h.app(|a| {
            a.drafts.active_mut().unwrap().attachments.push(
                OutAttachment {
                    filename: "report.txt".into(),
                    data: b"numbers".to_vec(),
                }
                .into(),
            );
            a.show_compose_attachments();
        });
    }

    fn outbox_row(h: &Harness) -> Option<(i64, db::Outbox)> {
        let db = Db::open(&h.path).unwrap();
        let ids = db.outbox_ids("").unwrap();
        let id = *ids.first()?;
        Some((id, db.outbox_entry(id).unwrap()?.state))
    }

    #[test]
    fn undo_before_the_build_ends_returns_the_exact_draft() {
        let h = Harness::new("undo");
        let jobs = hold_builds(&h);
        write_message(&h);
        let message_id = h.app(|a| {
            let id = a.drafts.active_id().unwrap();
            a.save_draft(id);
            a.drafts.active().unwrap().message_id.clone()
        });
        h.ui().invoke_send();
        h.frame();

        // Sent: the composer closed at once, the draft left Drafts and
        // waits in the outbox; the build runs.
        assert!(!h.ui().get_composing());
        assert_eq!(h.undo(), "Sending “Plans” in 5 s");
        assert_eq!(h.status(), "", "its own slot");
        let (row, state) = outbox_row(&h).expect("in the outbox");
        assert_eq!(state, db::Outbox::Pending);
        assert_eq!(jobs.borrow().len(), 1);
        let nav: Vec<(i32, i32)> = h
            .ui()
            .get_nav_items()
            .iter()
            .map(|n| (n.kind, n.count))
            .collect();
        assert!(nav.contains(&(4, 1)), "{nav:?}");
        assert!(nav.contains(&(3, 0)), "no saved drafts left: {nav:?}");
        // Frozen: no autosave writes it.
        h.app(|a| {
            assert!(a.drafts.active().is_none());
            a.autosave();
        });

        // Undo, before the build is done: the same draft comes back.
        h.ui().invoke_undo_clicked();
        h.frame();
        assert!(h.ui().get_composing());
        assert_eq!(h.ui().get_c_to(), "Ana <ana@example.org>");
        assert_eq!(h.ui().get_c_cc(), "cy@example.org");
        assert!(h.ui().get_c_show_cc());
        assert_eq!(h.ui().get_c_subject(), "Plans");
        assert_eq!(h.ui().get_c_body(), "Report attached.\nSee you");
        assert_eq!(h.ui().get_c_attachments().row_count(), 1);
        h.app(|a| {
            let d = a.drafts.active().unwrap();
            assert_eq!(
                (d.row, d.message_id.as_str()),
                (Some(row), message_id.as_str())
            );
            assert_eq!(d.attachments[0].file.data, b"numbers");
        });
        assert!(outbox_row(&h).is_none());

        // The late build is dropped, and the undo window's end does nothing.
        assert_eq!(finish_build(&h, &jobs), Ok(false));
        let seq = h.app(|a| a.send_seq);
        h.app(|a| a.send_due(row, seq));
        let db = Db::open(&h.path).unwrap();
        assert_eq!(db.draft_ids("").unwrap(), [row]);
        assert_eq!(db.count_outbox().unwrap(), 0);
        assert!(db.outbox_due(i64::MAX).unwrap().is_empty());
    }

    #[test]
    fn a_send_is_queued_once_built_and_past_its_undo_window() {
        let h = Harness::new("queue");
        let jobs = hold_builds(&h);

        // The window ends first: it waits for the build, Undo still works.
        write_message(&h);
        h.ui().invoke_send();
        h.frame();
        let (row, _) = outbox_row(&h).unwrap();
        let seq = h.app(|a| a.send_seq);
        h.app(|a| a.send_due(row, seq));
        assert_eq!(outbox_row(&h), Some((row, db::Outbox::Pending)));
        assert_ne!(h.undo(), "");
        assert_eq!(finish_build(&h, &jobs), Ok(true));
        // No account worker runs here, so it waits in the queue.
        assert_eq!(outbox_row(&h), Some((row, db::Outbox::Queued)));
        assert_eq!(h.status(), "", "the Undo offer went");
        let db = Db::open(&h.path).unwrap();
        assert_eq!(db.outbox_due(i64::MAX).unwrap().len(), 1);
        db.delete_draft(row).unwrap();

        // The build ends first: queued when the window ends.
        write_message(&h);
        h.ui().invoke_send();
        h.frame();
        let (row, _) = outbox_row(&h).unwrap();
        assert_eq!(finish_build(&h, &jobs), Ok(true));
        assert_eq!(outbox_row(&h), Some((row, db::Outbox::Pending)));
        let seq = h.app(|a| a.send_seq);
        h.app(|a| a.send_due(row, seq));
        assert_eq!(outbox_row(&h), Some((row, db::Outbox::Queued)));
        // Too late to undo once a send has it.
        assert!(db.claim_outbox(row).unwrap().is_some());
        h.app(|a| a.undo_send(row));
        assert_eq!(h.status(), "Too late to undo: the message is being sent.");
        assert!(!h.ui().get_composing());
    }

    #[test]
    fn the_undo_offer_counts_down_and_a_delivery_says_sent() {
        let h = Harness::new("countdown");
        let _jobs = hold_builds(&h);
        write_message(&h);
        // Checked and ticked by hand, without a frame: on a slow run the
        // real one-second timer would be due and overwrite the text.
        h.ui().invoke_send();
        let secs = with_app(|a| a.undo_delay.as_secs()).unwrap();
        assert_eq!(h.undo(), format!("Sending “Plans” in {secs} s"));
        let (row, _) = outbox_row(&h).unwrap();
        with_app(|a| a.count_down(row, Instant::now() + Duration::from_millis(8_500)));
        assert_eq!(h.undo(), "Sending “Plans” in 9 s", "the offer stays");
        // The window is over but the message is still being built.
        with_app(|a| a.count_down(row, Instant::now()));
        assert_eq!(h.undo(), "Sending\u{a0}…");
        // A status line never replaces it (its own slot); a move's offer
        // takes the slot and ends the countdown (the send goes on).
        h.app(|a| a.set_status("Saved", false));
        h.app(|a| a.count_down(row, Instant::now() + Duration::from_secs(5)));
        assert_eq!(h.undo(), "Sending “Plans” in 5 s");
        assert_eq!(h.status(), "Saved");
        h.app(|a| a.offer_undo("Moved to Archive", UndoOffer::Move(99)));
        h.app(|a| a.count_down(row, Instant::now() + Duration::from_secs(4)));
        assert_eq!(h.undo(), "Moved to Archive");
        h.app(|a| a.on_sent(None, Outcome::Delivered));
        assert_eq!(h.status(), "Sent");
    }

    #[test]
    fn send_guards_ask_once_and_send_anyway_goes_ahead() {
        let h = Harness::new("guard-bar");
        let jobs = hold_builds(&h);
        h.ui().invoke_compose();
        h.frame();
        type_into(h.ui(), ComposeField::To, "ana@example.org");
        type_into(h.ui(), ComposeField::Body, "Segue em anexo.");
        h.ui().invoke_send();
        h.frame();
        assert!(h.ui().get_composing());
        assert_eq!(
            h.ui().get_c_guard(),
            "No subject, and the text mentions an attachment but nothing is attached."
        );
        assert!(outbox_row(&h).is_none() && jobs.borrow().is_empty());
        // Typing dismisses it; the next Send asks again.
        type_into(h.ui(), ComposeField::Subject, "Contrato");
        assert_eq!(h.ui().get_c_guard(), "");
        h.ui().invoke_send();
        h.frame();
        assert_eq!(
            h.ui().get_c_guard(),
            "The text mentions an attachment, but nothing is attached."
        );
        h.ui().invoke_c_send_anyway();
        h.frame();
        assert!(!h.ui().get_composing());
        assert_eq!(outbox_row(&h).map(|r| r.1), Some(db::Outbox::Pending));
        assert_eq!(jobs.borrow().len(), 1);
    }

    /// Put an outbox row in `state` (Failed or Unconfirmed) the way a send
    /// would leave it.
    fn settled_row(h: &Harness, state: db::Outbox, error: &str) -> i64 {
        let db = Db::open(&h.path).unwrap();
        let account = db.accounts().unwrap()[0].id;
        let row = db
            .insert_draft(&db::DraftRow {
                account_id: account,
                to: "ana@example.org".into(),
                subject: "Plans".into(),
                body: "Hi Ana,\n\nThe **plans** for Monday.\n\n> quoted\n-- \nMe".into(),
                message_id: "<o1@example.org>".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(db.outbox_send(row).unwrap());
        assert!(db.store_raw(row, b"RAW", &AtomicBool::new(false)).unwrap());
        assert!(db.outbox_queue(row).unwrap());
        db.claim_outbox(row).unwrap().unwrap();
        db.outbox_result(row, state, None, error).unwrap();
        h.app(|a| a.outbox_changed());
        row
    }

    #[test]
    fn the_outbox_offers_edit_retry_and_discard() {
        let h = Harness::new("outbox-view");
        let refusal = "permanent error (550): 5.1.1 <ana@example.org>: User unknown";
        let failed = settled_row(&h, db::Outbox::Failed, refusal);
        let nav =
            |h: &Harness| -> Vec<i32> { h.ui().get_nav_items().iter().map(|n| n.kind).collect() };
        assert!(nav(&h).contains(&4));
        h.ui().invoke_nav_clicked(NavItem {
            kind: 4,
            ..Default::default()
        });
        h.frame();
        assert_eq!(h.ui().get_list_title(), "Outbox");
        assert_eq!(h.ui().get_selected_folder(), -2);
        let list = h.ui().get_rows();
        assert_eq!(list.row_count(), 1);
        let r = list.row_data(0).unwrap();
        assert_eq!(r.sender, "To: ana@example.org");
        let sentence = "ana@example.org doesn't exist. Check the address and send again.";
        assert_eq!(r.preview, sentence);
        h.ui().invoke_message_selected(r.id);
        h.frame();
        assert!(h.ui().get_o_show() && h.ui().get_o_problem());
        // A plain sentence for the SMTP class; the reply behind Details;
        // Edit leads (sending it again unchanged can't help); the first
        // lines of the text, without marks, quote or signature.
        assert_eq!(h.ui().get_o_status(), sentence);
        assert_eq!(h.ui().get_o_detail(), refusal);
        assert!(h.ui().get_o_permanent());
        assert_eq!(h.ui().get_o_body(), "Hi Ana,\nThe plans for Monday.");
        assert!(h.ui().get_o_can_edit() && h.ui().get_o_can_retry() && h.ui().get_o_can_discard());
        assert_eq!(h.ui().get_selected_id() as i64, failed);

        // Retry queues it again (no worker here, so it waits).
        h.ui().invoke_o_retry();
        h.frame();
        let db = Db::open(&h.path).unwrap();
        assert_eq!(
            db.outbox_entry(failed).unwrap().unwrap().state,
            db::Outbox::Queued
        );
        assert!(!h.ui().get_o_can_retry());

        // A refusal for now (out of tries): Retry leads.
        let busy = settled_row(&h, db::Outbox::Failed, "transient error (421): 4.7.0 later");
        h.ui().invoke_message_selected(busy as i32);
        h.frame();
        assert!(!h.ui().get_o_permanent());
        assert!(h.ui().get_o_status().contains("Komorebi stopped trying"));
        h.ui().invoke_o_retry();
        h.frame();

        // Retrying a message that may have gone out asks first.
        let unconfirmed = settled_row(&h, db::Outbox::Unconfirmed, "may have been sent");
        h.ui().invoke_message_selected(unconfirmed as i32);
        h.frame();
        assert!(!h.ui().get_o_permanent());
        h.ui().invoke_o_retry();
        h.frame();
        assert_eq!(h.ui().get_q_title(), "Send this message again?");
        h.ui().invoke_q_answer(false);
        h.frame();
        assert_eq!(h.ui().get_q_title(), "");
        assert_eq!(
            db.outbox_entry(unconfirmed).unwrap().unwrap().state,
            db::Outbox::Unconfirmed
        );
        h.ui().invoke_o_retry();
        h.ui().invoke_q_answer(true);
        h.frame();
        assert_eq!(
            db.outbox_entry(unconfirmed).unwrap().unwrap().state,
            db::Outbox::Queued
        );

        // Discard asks (the one discard sentence), then deletes it.
        h.ui().invoke_o_discard();
        h.frame();
        assert_eq!(h.ui().get_q_title(), "Discard this message?");
        assert_eq!(
            h.ui().get_q_text(),
            "It is deleted here and from your Drafts folder on the server, with its attachments."
        );
        h.ui().invoke_q_answer(true);
        h.frame();
        assert!(db.outbox_entry(unconfirmed).unwrap().is_none());
        assert!(!h.ui().get_o_show());

        // Edit takes the other back to Drafts and opens it.
        h.ui().invoke_message_selected(failed as i32);
        h.ui().invoke_o_edit();
        h.frame();
        assert!(h.ui().get_composing());
        assert_eq!(h.ui().get_c_subject(), "Plans");
        assert_eq!(db.count_outbox().unwrap(), 1, "the busy one is queued");
        assert_eq!(db.draft_ids("").unwrap(), [failed]);
        assert_eq!(h.ui().get_rows().row_count(), 1, "the busy one");
    }

    #[test]
    fn removing_an_account_with_unsent_mail_asks_first() {
        let h = Harness::new("remove-account");
        settled_row(&h, db::Outbox::Failed, "550");
        let account = h.app(|a| a.accounts[0].id);
        h.ui().invoke_remove_account(account as i32);
        h.frame();
        assert_eq!(h.ui().get_q_title(), "Remove me@example.org?");
        assert_eq!(
            h.ui().get_q_text(),
            "Removes this account and its downloaded mail from this computer. Mail on the \
             server is not affected. Its 1 unsent draft or Outbox message will be lost."
        );
        // Cancel keeps everything. (Accepting is not run here: removal
        // deletes the account's password from Windows Credential Manager.)
        h.ui().invoke_q_answer(false);
        h.frame();
        assert_eq!(h.ui().get_q_title(), "");
        assert_eq!(h.app(|a| a.accounts.len()), 1);
        assert_eq!(Db::open(&h.path).unwrap().count_outbox().unwrap(), 1);
    }

    #[test]
    fn removing_any_account_asks_first() {
        let h = Harness::new("remove-plain");
        let account = h.app(|a| a.accounts[0].id);
        h.ui().invoke_remove_account(account as i32);
        h.frame();
        assert_eq!(h.ui().get_q_title(), "Remove me@example.org?");
        assert_eq!(
            h.ui().get_q_text(),
            "Removes this account and its downloaded mail from this computer. Mail on the \
             server is not affected."
        );
        h.ui().invoke_q_answer(false);
        h.frame();
        assert_eq!(h.app(|a| a.accounts.len()), 1);
    }

    /// The app as the add-account tests want it: passwords go to a fake
    /// store (never Windows Credential Manager) and no worker starts.
    fn fake_store(h: &Harness) {
        h.app(|a| {
            a.store_password = Rc::new(|_, _| Ok(()));
            a.forget_password = Rc::new(|_| Ok(()));
            a.workers_on = false;
            a.verifier = |_, _, _, _, _, report| {
                crate::mail::errors::Step::ALL.into_iter().for_each(report);
                Ok(())
            };
        });
    }

    fn new_account(email: &str) -> NewAccount {
        let server = |host: &str| Server {
            host: host.into(),
            port: 993,
            security: Security::Tls,
        };
        NewAccount {
            imap_user: String::new(),
            smtp_user: String::new(),
            email: email.into(),
            name: String::new(),
            imap: server("imap.example.invalid"),
            smtp: server("smtp.example.invalid"),
            save_sent: true,
        }
    }

    #[test]
    fn the_title_bar_has_no_search_until_the_first_account() {
        let h = Harness::new("bar-search");
        let pushed = Rc::new(std::cell::Cell::new(usize::MAX));
        let seen = pushed.clone();
        h.ui()
            .on_title_bar_regions(move |_, rects, _| seen.set(rects.row_count()));
        // No account: no search, nothing passes through.
        h.ui().set_has_accounts(false);
        h.frame();
        assert_eq!(pushed.get(), 0);
        // With one: the search box and its scope.
        h.ui().set_has_accounts(true);
        h.frame();
        assert_eq!(pushed.get(), 2);
    }

    #[test]
    fn cancel_drops_a_late_check_answer_and_keeps_the_form() {
        let h = Harness::new("cancel-check");
        fake_store(&h);
        let db = Db::open(&h.path).unwrap();
        let before = db.accounts().unwrap().len();
        // The onboarding flow's Sign in (Add account opens it).
        h.ui().invoke_open_onboarding();
        h.frame();
        assert!(h.ui().get_ob_open());
        let form = AccountForm {
            provider: 0,
            email: "new@purelymail.com".into(),
            password: "pw".into(),
            ..Default::default()
        };
        // No provider chosen: refused before anything runs.
        h.ui().invoke_save_account(AccountForm {
            provider: -1,
            ..form.clone()
        });
        h.frame();
        assert_eq!(h.ui().get_a_error(), "Choose your provider.");
        assert!(!h.ui().get_a_busy());

        h.ui().invoke_save_account(form.clone());
        h.frame();
        assert!(h.ui().get_a_busy());
        let seq = h.app(|a| a.check_seq);
        // Cancel: the check's answer is dropped, the form stays.
        h.ui().invoke_cancel_check();
        h.frame();
        assert!(!h.ui().get_a_busy());
        assert!(h.ui().get_ob_open());
        assert!(h.ui().get_a_test().iter().next().is_none());
        // The late answer adds nothing, even though every step passed.
        let late = new_account("new@purelymail.com");
        h.app(|a| {
            a.finish_check(
                seq,
                Ok(()),
                Some(Box::new(move |app: &mut App| {
                    app.finish_add_account(late, String::new(), "pw".into(), (None, None))
                })),
            )
        });
        assert_eq!(db.accounts().unwrap().len(), before);
        assert!(h.ui().get_ob_open());
        assert_eq!(h.ui().get_ob_step(), 1);

        // Esc while a check runs drops it too (Back and Esc stay usable).
        h.ui().invoke_save_account(form);
        h.frame();
        assert!(h.ui().get_a_busy());
        let seq = h.app(|a| a.check_seq);
        h.keys(&[], &SharedString::from(Key::Escape));
        assert!(!h.ui().get_a_busy());
        assert!(h.app(|a| a.check_seq) > seq);
        assert_eq!(db.accounts().unwrap().len(), before);
    }

    #[test]
    fn an_added_account_goes_on_to_make_it_yours_and_its_first_download() {
        let h = Harness::new("land-inbox");
        fake_store(&h);
        h.ui().invoke_open_onboarding();
        h.frame();
        let before = h.ui().get_selected_folder();
        h.app(|a| {
            a.finish_add_account(
                new_account("ana.silva@example.net"),
                String::new(),
                "pw".into(),
                (None, None),
            )
        });
        // Make it yours, the name from the address; no "Added …" line.
        assert_eq!(h.ui().get_ob_step(), 3);
        assert!(h.ui().get_ob_open());
        assert_eq!(h.ui().get_ob_name(), "Ana Silva");
        assert_eq!(h.ui().get_ob_address(), "ana.silva@example.net");
        assert_eq!(h.status(), "");
        let db = Db::open(&h.path).unwrap();
        let id = db
            .accounts()
            .unwrap()
            .iter()
            .find(|a| a.email == "ana.silva@example.net")
            .unwrap()
            .id;
        // No folders yet: the view stays until its Inbox is known.
        assert_eq!(h.ui().get_selected_folder(), before);

        // The folders arrive: the new Inbox opens behind the flow, and its
        // empty list shows the download starting.
        let inbox = db
            .upsert_folder(id, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        h.app(|a| a.on_event(Event::FolderChanged { folder_id: None }));
        assert_eq!(h.ui().get_selected_folder(), inbox as i32);
        assert_eq!(h.ui().get_list_fresh(), 1);
        assert_eq!(
            h.ui().get_list_empty(),
            "Downloading your mail\u{a0}… Newest messages arrive first."
        );
        // Syncing: the flow's bar is indeterminate until a folder reports
        // numbers (a small mailbox never does).
        h.app(|a| {
            a.on_event(Event::Syncing {
                account_id: id,
                active: true,
                error: None,
            })
        });
        assert_eq!(h.ui().get_ob_sync(), 1);
        // Its progress, determinate (no overall percentage).
        h.app(|a| {
            a.on_event(Event::Progress {
                account_id: id,
                folder_id: inbox,
                done: 1234,
                total: 5000,
            })
        });
        assert_eq!(h.ui().get_list_fresh(), 2);
        assert!((h.ui().get_list_fresh_value() - 0.2468).abs() < 0.001);
        let text = "Downloading your mail\u{a0}… 1,234 of 5,000. Newest messages arrive first.";
        assert_eq!(h.ui().get_list_empty(), text);
        assert_eq!(h.ui().get_ob_sync(), 2);
        assert_eq!(h.ui().get_ob_sync_text(), text);
        // A late, smaller count for the same folder never moves it back.
        h.app(|a| {
            a.on_event(Event::Progress {
                account_id: id,
                folder_id: inbox,
                done: 1000,
                total: 5000,
            })
        });
        assert!((h.ui().get_ob_sync_value() - 0.2468).abs() < 0.001);
        // The first pass ends: an empty folder says so.
        h.app(|a| {
            a.on_event(Event::Syncing {
                account_id: id,
                active: false,
                error: None,
            })
        });
        assert_eq!(h.status(), "");
        assert_eq!(h.ui().get_list_fresh(), 0);
        assert_eq!(h.ui().get_list_empty(), "This folder is empty");
        assert_eq!(h.ui().get_ob_sync(), 3);

        // Done writes the name and signature; then the finish.
        h.ui()
            .invoke_onboard_personalise(true, "Ana Silva".into(), "Ana\nExample".into());
        let saved = db.accounts().unwrap();
        let a = saved.iter().find(|a| a.id == id).unwrap();
        assert_eq!(
            (a.name.as_str(), a.signature.as_str()),
            ("Ana Silva", "Ana\nExample")
        );
        assert_eq!(h.ui().get_ob_step(), 4);
        // Open your inbox: the flow closes on it.
        h.ui().invoke_onboard_open_inbox();
        h.frame();
        assert!(!h.ui().get_ob_open());
        assert_eq!(h.ui().get_ob_step(), 0);
        assert_eq!(h.ui().get_selected_folder(), inbox as i32);
    }

    #[test]
    fn skip_writes_nothing_and_continue_never_looks_up_a_preset() {
        let h = Harness::new("ob-skip");
        fake_store(&h);
        h.ui().invoke_open_onboarding();
        h.frame();
        let fields = crate::ServerFields {
            imap_port: "993".into(),
            smtp_port: "465".into(),
            ..Default::default()
        };
        // Not an address, then one already here: said under the field.
        h.ui()
            .invoke_onboard_continue("ana@".into(), fields.clone());
        assert_eq!(h.ui().get_ob_email_problem(), 1);
        assert_eq!(h.ui().get_ob_step(), 1);
        let taken = h.app(|a| a.accounts[0].email.clone());
        h.ui()
            .invoke_onboard_continue(taken.to_uppercase().as_str().into(), fields.clone());
        assert_eq!(h.ui().get_ob_email_problem(), 2);
        // A preset's address: its settings are built in, nothing is looked
        // up (no lookup started, none running).
        let finds = h.app(|a| a.find_seq);
        h.ui()
            .invoke_onboard_continue("ana@purelymail.com".into(), fields);
        assert_eq!(
            h.app(|a| a.find_seq),
            finds + 1,
            "only the reset, no lookup"
        );
        assert!(!h.ui().get_a_finding());
        assert_eq!(h.ui().get_ob_chip(), 2);
        assert_eq!(h.ui().get_ob_provider(), 0);
        assert!(!h.ui().get_ob_manual());
        assert_eq!(h.ui().get_ob_step(), 2);

        // Added; Skip leaves the account as it was saved.
        h.app(|a| {
            a.finish_add_account(
                new_account("ana@purelymail.com"),
                String::new(),
                "pw".into(),
                (None, None),
            )
        });
        let db = Db::open(&h.path).unwrap();
        let before = db
            .accounts()
            .unwrap()
            .into_iter()
            .find(|a| a.email == "ana@purelymail.com")
            .unwrap();
        h.ui()
            .invoke_onboard_personalise(false, "Someone".into(), "Sig".into());
        let after = db
            .accounts()
            .unwrap()
            .into_iter()
            .find(|a| a.email == "ana@purelymail.com")
            .unwrap();
        assert_eq!(format!("{after:?}"), format!("{before:?}"));
        assert_eq!(after.name, "");
        assert_eq!(h.ui().get_ob_step(), 4);
    }

    #[test]
    fn a_rejected_password_sets_the_username_path_by_provider() {
        let h = Harness::new("rejected");
        fake_store(&h);
        let reject = |h: &Harness, form: AccountForm| {
            h.ui().invoke_test_account(form);
            h.frame();
            let seq = h.app(|a| a.check_seq);
            let say = Say::Rejected {
                host: "mail.example.net".into(),
                login: "ana@example.net".into(),
            };
            h.app(|a| {
                a.finish_check(
                    seq,
                    Err(CheckFailure {
                        step: Some(crate::mail::errors::Step::ImapSignIn.index()),
                        text: CheckText::Say(say),
                        review: None,
                    }),
                    None,
                )
            });
            assert!(h.ui().get_a_rejected());
            h.ui().get_a_error().to_string()
        };
        let form = AccountForm {
            email: "ana@example.net".into(),
            password: "pw".into(),
            ..Default::default()
        };
        // Other server: the username advice too.
        let other = reject(
            &h,
            AccountForm {
                provider: providers::PRESETS.len() as i32,
                imap_host: "mail.example.net".into(),
                smtp_host: "mail.example.net".into(),
                imap_port: "993".into(),
                smtp_port: "465".into(),
                ..form.clone()
            },
        );
        assert!(other.ends_with("set the username."), "{other}");
        // A preset on a custom domain: the app password, then the other
        // preset.
        let preset = reject(
            &h,
            AccountForm {
                provider: 0,
                ..form.clone()
            },
        );
        assert_eq!(
            preset,
            "mail.example.net rejected the password for ana@example.net. If sign-in uses two steps, \
             create an app password. Is your mail at Postale.io? Change Provider."
        );
        // A new check clears the state.
        h.ui().invoke_test_account(AccountForm {
            provider: 1,
            ..form
        });
        h.frame();
        assert!(!h.ui().get_a_rejected());
    }

    /// The settings form as the window would send it for account `id`.
    fn edit_form(h: &Harness, id: i64) -> AccountForm {
        h.ui().invoke_edit_account(id as i32);
        h.frame();
        assert!(h.ui().get_show_settings());
        assert_eq!(h.ui().get_a_edit_id(), id as i32);
        h.ui().get_a_edit()
    }

    #[test]
    fn test_connection_shows_its_steps_and_saves_nothing() {
        use crate::mail::errors::Step;
        let h = Harness::new("test-connection");
        // A check that passes every step, whatever the servers.
        h.app(|a| {
            a.verifier = |_, _, _, _, _, report| {
                Step::ALL.into_iter().for_each(report);
                Ok(())
            };
        });
        let db = Db::open(&h.path).unwrap();
        let before = db.accounts().unwrap().len();
        let form = AccountForm {
            provider: providers::PRESETS.len() as i32,
            name: "New".into(),
            email: "new@example.net".into(),
            password: "pw".into(),
            imap_host: "imap.example.net".into(),
            imap_port: "993".into(),
            smtp_host: "smtp.example.net".into(),
            smtp_port: "465".into(),
            ..Default::default()
        };

        // Without a password there is nothing to test.
        h.ui().invoke_test_account(AccountForm {
            password: "".into(),
            ..form.clone()
        });
        h.frame();
        assert_eq!(h.ui().get_a_error(), "Enter the password to test.");
        assert!(!h.ui().get_a_busy());

        // Running: the first step is under way.
        h.ui().invoke_test_account(form.clone());
        h.frame();
        assert!(h.ui().get_a_busy());
        let states = |h: &Harness| h.ui().get_a_test().iter().collect::<Vec<_>>();
        assert_eq!(states(&h), [1, 0, 0, 0, 0, 0, 0]);
        // An answer from an older check is dropped.
        let seq = h.app(|a| a.check_seq);
        h.app(|a| a.finish_check(seq - 1, Ok(()), None));
        assert!(h.ui().get_a_busy());

        // All passed: said so, and still nothing saved.
        h.app(|a| a.finish_check(seq, Ok(()), None));
        assert!(!h.ui().get_a_busy());
        assert_eq!(states(&h), [2; 7]);
        assert!(h.ui().get_a_ok().contains("Nothing was saved"));
        assert_eq!(h.ui().get_a_error(), "");
        assert_eq!(db.accounts().unwrap().len(), before);

        // A failure highlights its step, skips the rest and shows the
        // sentence only.
        h.ui().invoke_test_account(form);
        h.frame();
        let seq = h.app(|a| a.check_seq);
        let say = Say::Rejected {
            host: "imap.example.net".into(),
            login: "new@example.net".into(),
        };
        let text = text::say(&say);
        h.app(|a| {
            a.finish_check(
                seq,
                Err(CheckFailure {
                    step: Some(Step::ImapSignIn.index()),
                    text: CheckText::Say(say),
                    review: None,
                }),
                None,
            )
        });
        assert_eq!(states(&h), [2, 2, 2, 3, 4, 4, 4]);
        assert_eq!(h.ui().get_a_error(), text.as_str());
        assert_eq!(h.ui().get_a_ok(), "");
        assert_eq!(db.accounts().unwrap().len(), before);

        // Closing the dialog forgets all of it.
        h.ui().invoke_settings_closed();
        assert!(states(&h).is_empty());
        assert_eq!(h.ui().get_a_error(), "");
    }

    fn fixture_der() -> Vec<u8> {
        native_tls::Certificate::from_pem(include_bytes!("../../tests/fixtures/tls/test-cert.pem"))
            .unwrap()
            .to_der()
            .unwrap()
    }

    fn manual_form(h: &Harness, id: i64) -> AccountForm {
        AccountForm {
            provider: providers::PRESETS.len() as i32,
            ..edit_form(h, id)
        }
    }

    #[test]
    fn a_certificate_is_pinned_only_after_the_user_confirms_it() {
        use crate::mail::pin::Kind;
        let h = Harness::new("review-cert");
        let server = Server {
            host: "mail.example.org".into(),
            port: 993,
            security: Security::Tls,
        };
        let der = fixture_der();
        let seq = h.app(|a| {
            a.check_seq += 1;
            a.check_seq
        });
        // An answer from an older check is dropped.
        h.app(|a| a.finish_review(seq - 1, Kind::Imap, server.clone(), Ok(der.clone())));
        assert_eq!(h.ui().get_q_title(), "");

        h.app(|a| a.finish_review(seq, Kind::Imap, server.clone(), Ok(der.clone())));
        h.frame();
        assert_eq!(h.ui().get_q_title(), "Trust this certificate?");
        assert_eq!(
            h.ui().get_q_text(),
            "mail.example.org:993 presented a certificate that Windows doesn't trust. Trust this certificate for mail.example.org only?"
        );
        assert_eq!(h.ui().get_q_ok(), "Trust this certificate");
        // A security question: Cancel is the default, and the whole
        // fingerprint is shown.
        assert!(h.ui().get_q_destructive());
        let code = h.ui().get_q_code();
        let fingerprint = "F7:AE:3F:F2:B3:50:84:AB:53:1A:A9:1D:FC:12:AC:BB:67:A7:85:F0:69:9D:12:AB:77:76:7F:2C:6A:77:1B:9A";
        assert_eq!(code.replace('\n', ":"), fingerprint);
        assert_eq!(code.lines().count(), 4);
        assert_eq!(h.ui().get_q_code_label(), "SHA-256 fingerprint");
        // Copy is the IconButton beside it, not a third button.
        assert_eq!(h.ui().get_q_alt(), "");
        assert!(h.ui().get_q_detail().contains("Self-signed"));
        assert!(h.ui().get_q_note().contains("openssl"));

        // Cancel pins nothing.
        h.ui().invoke_q_answer(false);
        h.frame();
        assert_eq!(h.app(|a| a.form_pins.blobs()), (None, None));
        assert!(!h.ui().get_a_pinned_imap());

        // Trust pins it in the form; nothing is saved.
        h.app(|a| a.finish_review(seq, Kind::Imap, server.clone(), Ok(der.clone())));
        h.ui().invoke_q_answer(true);
        h.frame();
        assert_eq!(h.app(|a| a.form_pins.blobs()), (Some(der.clone()), None));
        assert!(
            h.ui()
                .get_a_cert_imap()
                .starts_with("Certificate: pinned (SHA-256 F7:AE:3F:F2")
        );
        assert!(h.ui().get_a_pinned_imap() && !h.ui().get_a_pinned_smtp());
        assert!(
            h.ui()
                .get_a_ok()
                .starts_with("Certificate trusted for mail.example.org.")
        );
        let db = Db::open(&h.path).unwrap();
        assert!(db.accounts().unwrap()[0].imap_pin.is_none());

        // Forgetting it brings the question back with the old fingerprint.
        h.ui().invoke_forget_certificate(1);
        h.frame();
        assert_eq!(h.app(|a| a.form_pins.blobs()), (None, None));
        assert!(!h.ui().get_a_pinned_imap());
        h.app(|a| a.finish_review(seq, Kind::Imap, server.clone(), Ok(der.clone())));
        assert_eq!(
            h.ui().get_q_old(),
            crate::app::cert::group_fingerprint(fingerprint)
        );
        assert_eq!(h.ui().get_q_old_label(), "Saved before (SHA-256)");
        h.ui().invoke_q_answer(false);
        assert_eq!(h.ui().get_q_title(), "");

        // A certificate that could not be read says so.
        let unread = cert::Unreadable::Unread {
            address: "imap.example.net:993".into(),
        };
        h.app(|a| a.finish_review(seq, Kind::Imap, server, Err(unread)));
        assert_eq!(
            h.ui().get_a_error(),
            "Couldn't read the certificate of imap.example.net:993. Check the server name, the port and the security setting, then try again."
        );
        assert!(!h.ui().get_a_busy());
    }

    #[test]
    fn an_untrusted_certificate_offers_the_review_and_a_changed_one_does_not() {
        use crate::mail::pin::Kind;
        let h = Harness::new("review-offer");
        let fail = |review| CheckFailure {
            step: Some(2),
            text: CheckText::NoSavedPassword,
            review,
        };
        let id = h.app(|a| a.accounts[0].id);
        h.ui().invoke_test_account(AccountForm {
            password: "pw".into(),
            ..manual_form(&h, id)
        });
        let seq = h.app(|a| a.check_seq);
        h.app(|a| a.finish_check(seq, Err(fail(Some(Kind::Smtp))), None));
        assert_eq!(h.ui().get_a_review(), 2);
        h.app(|a| a.finish_check(seq, Err(fail(Some(Kind::Imap))), None));
        assert_eq!(h.ui().get_a_review(), 1);
        h.app(|a| a.finish_check(seq, Err(fail(None)), None));
        assert_eq!(h.ui().get_a_review(), 0);
    }

    #[test]
    fn saved_pins_follow_the_servers_they_were_confirmed_for() {
        let h = Harness::new("pins-follow");
        let id = h.app(|a| a.accounts[0].id);
        let der = fixture_der();
        let db = Db::open(&h.path).unwrap();
        let mut account = db.accounts().unwrap().remove(0);
        account.imap_pin = Some(der.clone());
        db.update_account(&account).unwrap();
        h.app(|a| a.load_accounts().unwrap());

        let form = manual_form(&h, id);
        assert!(h.ui().get_a_pinned_imap());
        let old = h.app(|a| a.accounts[0].clone());
        let pins = |h: &Harness, f: &AccountForm| {
            let (i, s) = form_servers(f).unwrap();
            h.app(|a| a.pins_for(f, (&i, &s), Some(&old)))
        };
        // The same servers, whatever else was edited: kept.
        let kept = AccountForm {
            name: "Other".into(),
            imap_user: "alice".into(),
            smtp_own_login: true,
            smtp_user: "bob".into(),
            password: "new".into(),
            ..form.clone()
        };
        assert_eq!(pins(&h, &kept), (Some(der.clone()), None));
        // Another host, port or security: gone, and typing the old server
        // back does not bring it back.
        for changed in [
            AccountForm {
                imap_host: "other.example.org".into(),
                ..form.clone()
            },
            AccountForm {
                imap_port: "994".into(),
                ..form.clone()
            },
            AccountForm {
                imap_starttls: true,
                ..form.clone()
            },
        ] {
            assert_eq!(pins(&h, &changed), (None, None));
            assert_eq!(pins(&h, &form), (None, None));
            h.app(|a| a.seed_pins(&old));
        }
        // A preset has no pins.
        let preset = AccountForm {
            provider: 0,
            ..form.clone()
        };
        assert_eq!(pins(&h, &preset), (None, None));
        h.app(|a| a.seed_pins(&old));

        // A save that changes only the name keeps the saved certificate.
        h.ui().invoke_save_account(AccountForm {
            name: "Renamed".into(),
            ..edit_form(&h, id)
        });
        h.frame();
        let saved = Db::open(&h.path).unwrap().accounts().unwrap().remove(0);
        assert_eq!(saved.name, "Renamed");
        assert_eq!(saved.imap_pin, Some(der));
        assert_eq!(saved.smtp_pin, None);
    }

    #[test]
    fn a_pin_round_trips_with_a_confirmed_edit_and_goes_with_the_account() {
        let h = Harness::new("pin-saved");
        let id = h.app(|a| a.accounts[0].id);
        let der = fixture_der();
        let old = h.app(|a| a.accounts[0].clone());
        let edited = Account {
            imap_pin: Some(der.clone()),
            smtp_pin: Some(der.clone()),
            ..old.clone()
        };
        h.app(|a| a.finish_update_account(old, edited, None));
        let db = Db::open(&h.path).unwrap();
        let a = db.accounts().unwrap().remove(0);
        assert_eq!(
            (a.imap_pin.as_deref(), a.smtp_pin.as_deref()),
            (Some(&der[..]), Some(&der[..]))
        );
        db.remove_account(id).unwrap();
        assert!(db.accounts().unwrap().is_empty());
    }

    #[test]
    fn editing_an_account_keeps_its_mail_and_never_shows_the_password() {
        let h = Harness::new("edit-account");
        let id = h.app(|a| a.accounts[0].id);
        let form = edit_form(&h, id);
        assert_eq!(form.email, "me@example.org");
        assert_eq!((form.name.as_str(), form.save_sent), ("Me", true));
        assert_eq!(form.imap_host, "imap.example.invalid");
        assert_eq!(
            (form.imap_port.as_str(), form.imap_starttls),
            ("993", false)
        );
        // Manual servers, and no password: empty keeps the saved one.
        assert_eq!(form.provider, providers::PRESETS.len() as i32);
        assert_eq!(form.password, "");
        // No separate sign-in names: both are the address.
        assert_eq!(
            (
                form.imap_user.as_str(),
                form.smtp_user.as_str(),
                form.smtp_own_login
            ),
            ("", "", false)
        );

        // Name, signature and the Sent choice save at once (nothing to
        // check with the server). The address can't change.
        h.ui().invoke_save_account(AccountForm {
            name: "Me Myself".into(),
            email: "someone@else.org".into(),
            save_sent: false,
            signature: "Me\nExample Org".into(),
            ..form
        });
        h.frame();
        let db = Db::open(&h.path).unwrap();
        let a = db.accounts().unwrap().remove(0);
        assert_eq!(a.email, "me@example.org");
        assert_eq!((a.name.as_str(), a.save_sent), ("Me Myself", false));
        assert_eq!(a.signature, "Me\nExample Org");
        assert_eq!(
            db.message_ids(&[db.folders().unwrap()[0].id], "").unwrap(),
            [h.message]
        );
        assert!(!h.ui().get_show_settings());
        assert_eq!(h.ui().get_a_edit_id(), 0);
        assert_eq!(h.status(), "Saved me@example.org.");
        assert_eq!(h.app(|a| a.accounts[0].name.clone()), "Me Myself");
        assert!(
            h.ui()
                .get_c_from_options()
                .row_data(0)
                .unwrap()
                .starts_with("Me Myself <")
        );

        // A bad host name is refused before anything is checked or saved.
        let form = edit_form(&h, id);
        h.ui().invoke_save_account(AccountForm {
            smtp_host: "https://smtp.example.org/".into(),
            ..form
        });
        h.frame();
        assert!(
            h.ui()
                .get_a_error()
                .starts_with("Enter the server name only")
        );
        assert_eq!(db.accounts().unwrap()[0].smtp.host, "smtp.example.invalid");

        // A bad port is refused before anything is checked or saved.
        let form = edit_form(&h, id);
        h.ui().invoke_save_account(AccountForm {
            imap_port: "99999".into(),
            ..form
        });
        h.frame();
        assert_eq!(h.ui().get_a_error(), "Enter a valid IMAP port.");
        assert!(!h.ui().get_a_busy());
        assert_eq!(db.accounts().unwrap()[0].imap.port, 993);
        // Closing the dialog forgets the form.
        h.ui().invoke_settings_closed();
        assert_eq!(h.ui().get_a_edit_id(), 0);
        assert_eq!(h.ui().get_a_edit().email, "");
    }

    #[test]
    fn new_mail_and_replies_start_with_the_signature() {
        let h = Harness::new("signature");
        let db = Db::open(&h.path).unwrap();
        let me = db.accounts().unwrap().remove(0);
        // Named signatures, each its account's two defaults.
        db.adopt_signature_text(me.id, "Me\nExample Org").unwrap();
        let work = db
            .add_account(&NewAccount {
                imap_user: String::new(),
                smtp_user: String::new(),
                email: "me@work.example".into(),
                name: "Me".into(),
                imap: me.imap.clone(),
                smtp: me.smtp.clone(),
                save_sent: true,
            })
            .unwrap();
        assert_eq!(db.accounts().unwrap()[1].id, work);
        db.adopt_signature_text(work, "Me at Work").unwrap();
        h.app(|a| a.load_accounts().unwrap());

        h.app(|a| a.compose_new());
        assert_eq!(h.ui().get_c_body(), "\n\n-- \nMe\nExample Org");
        // Only the signature: closing keeps nothing.
        h.ui().invoke_c_close();
        h.frame();
        assert_eq!(db.count_drafts().unwrap(), 0);
        // Nor does another From account or High importance on it.
        h.app(|a| a.compose_new());
        h.ui().invoke_c_from_changed(1);
        h.ui().invoke_c_set_importance(1);
        h.frame();
        assert_eq!(h.ui().get_c_body(), "\n\n-- \nMe at Work");
        assert_eq!(db.count_drafts().unwrap(), 0);
        h.ui().invoke_c_close();
        h.frame();
        assert_eq!(db.count_drafts().unwrap(), 0);
        assert_eq!(h.app(|a| a.drafts.len()), 0);

        // Another From account swaps the untouched signature.
        h.app(|a| a.compose_new());
        type_into(h.ui(), ComposeField::Body, "Hi\n\n-- \nMe\nExample Org");
        h.ui().invoke_c_from_changed(1);
        h.frame();
        assert_eq!(h.ui().get_c_body(), "Hi\n\n-- \nMe at Work");
        assert_eq!(
            h.app(|a| a.drafts.active().unwrap().body.clone()),
            "Hi\n\n-- \nMe at Work"
        );
        h.app(|a| a.discard_confirmed());

        // A reply: above the quote, with the cursor at the top.
        let id = h.message;
        h.app(|a| a.select_message(id));
        h.app(|a| a.reply(false));
        h.keys(&[], "O");
        h.keys(&[], "k");
        assert_eq!(h.ui().get_c_body(), "Ok\n\n-- \nMe\nExample Org");
        let body = h.app(|a| a.drafts.active().unwrap().full_body());
        assert!(
            body.starts_with("Ok\n\n-- \nMe\nExample Org\n\nOn "),
            "{body}"
        );
        assert!(body.ends_with("> Noon?"), "{body}");
    }

    #[test]
    fn sends_cut_short_by_the_last_exit_go_out_at_start() {
        let h = Harness::new("resume");
        let jobs = hold_builds(&h);
        let db = Db::open(&h.path).unwrap();
        let account = db.accounts().unwrap()[0].id;
        let draft = |subject: &str| {
            db.insert_draft(&db::DraftRow {
                account_id: account,
                to: "ana@example.org".into(),
                subject: subject.into(),
                message_id: format!("<{subject}@example.org>"),
                ..Default::default()
            })
            .unwrap()
        };
        // Built before the exit; not built; not built and its attachment
        // copy is gone.
        let built = draft("built");
        assert!(db.outbox_send(built).unwrap());
        assert!(
            db.store_raw(built, b"RAW", &AtomicBool::new(false))
                .unwrap()
        );
        let unbuilt = draft("unbuilt");
        assert!(db.outbox_send(unbuilt).unwrap());
        let lost = draft("lost");
        let file = db
            .add_draft_file(lost, "a.txt", "text/plain", b"a")
            .unwrap();
        assert!(db.outbox_send(lost).unwrap());
        std::fs::remove_file(
            db::drafts_dir(&h.path)
                .join(lost.to_string())
                .join(file.id.to_string()),
        )
        .unwrap();

        h.app(|a| a.resume_outbox());
        let state = |row| db.outbox_entry(row).unwrap().map(|e| e.state);
        assert_eq!(state(built), Some(db::Outbox::Queued));
        assert_eq!(state(unbuilt), Some(db::Outbox::Pending));
        // Never sent without its attachment: back to Drafts.
        assert_eq!(state(lost), None);
        assert_eq!(db.draft_ids("").unwrap(), [lost]);
        assert!(h.status().contains("back in Drafts"), "{}", h.status());
        // The rebuilt one is queued as soon as it is built (no undo window).
        assert_eq!(jobs.borrow().len(), 1);
        assert_eq!(finish_build(&h, &jobs), Ok(true));
        assert_eq!(state(unbuilt), Some(db::Outbox::Queued));
        assert_eq!(db.outbox_due(i64::MAX).unwrap().len(), 2);
    }

    #[test]
    fn multi_select_acts_on_every_selected_message() {
        let h = Harness::new("multi");
        let mut db = Db::open(&h.path).unwrap();
        let (account, inbox) = h.app(|a| (a.accounts[0].id, a.folders[0].id));
        let headers: Vec<NewHeader> = (2..=6)
            .map(|uid| NewHeader {
                uid,
                subject: format!("m{uid}"),
                date: 1_700_000_000 + i64::from(uid),
                ..Default::default()
            })
            .collect();
        db.insert_headers(account, inbox, &headers).unwrap();
        db.upsert_folder(account, "Trash", "Trash", 0, Role::Trash)
            .unwrap();
        h.app(|a| {
            a.load_accounts().unwrap();
            a.reload_rows();
        });
        let ids: Vec<i64> = h.app(|a| (0..a.rows.len()).filter_map(|i| a.rows.id_at(i)).collect());
        assert_eq!(ids.len(), 6);
        let ui = h.ui();
        let count = || ui.get_selected_count();
        let key = |k: Key| SharedString::from(k);
        let rows_selected = || {
            h.app(|a| {
                (0..a.rows.len())
                    .filter(|&i| a.rows.row_data(i).unwrap().selected)
                    .count()
            })
        };

        ui.invoke_message_selected(ids[0] as i32);
        h.frame();
        assert!(ui.get_has_message());
        assert_eq!(count(), 1);
        // Ctrl+click adds a row without opening it (or marking it read).
        ui.invoke_message_toggled(ids[2] as i32);
        h.frame();
        assert!(!ui.get_has_message());
        assert_eq!(count(), 2);
        assert_eq!(rows_selected(), 2);
        assert!(!h.app(|a| a.db.message(ids[2]).unwrap().unwrap().seen));
        // Shift+click and Shift+Down select from the row Ctrl+clicked.
        ui.invoke_message_extended(ids[4] as i32);
        h.frame();
        assert_eq!(count(), 3);
        ui.invoke_focus_list();
        h.keys(&[Key::Shift], &key(Key::DownArrow));
        assert_eq!(count(), 4);
        assert_eq!(ui.get_selected_index(), 5);
        // Ctrl+A selects the whole view; the buttons act on all of it.
        h.keys(&[Key::Control], "a");
        assert_eq!(count(), 6);
        assert_eq!(rows_selected(), 6);
        ui.invoke_mark_read(false);
        ui.invoke_mark_flagged(true);
        h.frame();
        let briefs = h.app(|a| a.db.briefs(&ids).unwrap());
        assert!(briefs.iter().all(|m| !m.seen && m.flagged));
        assert!(h.app(|a| a.rows.row_data(1).unwrap().unread));
        // Ctrl+Q: all read, since some were unread.
        h.keys(&[Key::Control], "q");
        assert!(h.app(|a| a.db.briefs(&ids).unwrap()).iter().all(|m| m.seen));
        // Esc clears.
        h.keys(&[], &key(Key::Escape));
        assert_eq!(count(), 0);
        assert_eq!(rows_selected(), 0);
        assert!(!ui.get_has_message());

        // Delete moves the selection to Trash and opens the next message.
        ui.invoke_message_selected(ids[1] as i32);
        ui.invoke_message_extended(ids[3] as i32);
        h.frame();
        assert_eq!(count(), 3);
        h.keys(&[], &key(Key::Delete));
        assert!(ui.get_has_message());
        assert_eq!(count(), 1);
        h.app(|a| {
            assert_eq!(a.open.as_ref().map(|o| o.msg.id), Some(ids[4]));
            assert_eq!(a.rows.len(), 3);
            // Held for Undo: out of the list, still in the cache until
            // the job goes.
            assert!(a.db.message(ids[2]).unwrap().is_some());
            a.flush_held();
            assert!(a.db.message(ids[2]).unwrap().is_none());
        });
        // A plain Down moves on from there.
        h.keys(&[], &key(Key::DownArrow));
        assert_eq!(h.app(|a| a.open.as_ref().map(|o| o.msg.id)), Some(ids[5]));
    }

    /// The harness with five more messages (UIDs 2-6, unread) and the
    /// given extra folders; returns the listed ids, newest first.
    fn more_messages(h: &Harness, folders: &[(&str, Role)]) -> Vec<i64> {
        let mut db = Db::open(&h.path).unwrap();
        let (account, inbox) = h.app(|a| (a.accounts[0].id, a.folders[0].id));
        let headers: Vec<NewHeader> = (2..=6)
            .map(|uid| NewHeader {
                uid,
                subject: format!("m{uid}"),
                date: 1_700_000_000 + i64::from(uid),
                ..Default::default()
            })
            .collect();
        db.insert_headers(account, inbox, &headers).unwrap();
        for &(name, role) in folders {
            db.upsert_folder(account, name, name, 0, role).unwrap();
        }
        h.app(|a| {
            a.load_accounts().unwrap();
            a.reload_rows();
            (0..a.rows.len()).filter_map(|i| a.rows.id_at(i)).collect()
        })
    }

    fn held_ids(h: &Harness) -> Vec<i64> {
        h.app(|a| {
            let mut ids: Vec<i64> = a.held.iter().flat_map(|x| x.ids.iter().copied()).collect();
            ids.sort_unstable();
            ids
        })
    }

    #[test]
    fn archive_and_delete_wait_for_their_undo_window() {
        let h = Harness::new("held");
        let ids = more_messages(&h, &[("Archive", Role::Archive), ("Trash", Role::Trash)]);
        let ui = h.ui();
        let open = || h.app(|a| a.open.as_ref().map(|o| o.msg.id));
        // The Inbox's count (one account: no All inboxes row).
        let unread = || {
            h.app(|a| {
                a.nav
                    .iter()
                    .find(|r| r.kind == 2 && r.label == "Inbox")
                    .unwrap()
                    .unread
            })
        };
        let listed = |id| h.app(|a| a.rows.index_of(id).is_some());
        let cached = |id| h.app(|a| a.db.message(id).unwrap().is_some());
        assert_eq!(unread(), 6);

        // Archive the open message: it leaves the list (and the count), the
        // next one opens, Undo is offered; nothing changed in the cache.
        ui.invoke_message_selected(ids[1] as i32);
        h.frame();
        assert_eq!(unread(), 5);
        ui.invoke_archive();
        h.frame();
        assert!(!listed(ids[1]));
        assert_eq!(open(), Some(ids[2]));
        assert_eq!(h.undo(), "Moved to Archive");
        assert_eq!(h.status(), "", "the Undo offer has its own slot");
        assert!(cached(ids[1]));
        assert_eq!(held_ids(&h), [ids[1]]);
        // The job waits: one Move to Archive, held.
        h.app(|a| {
            let archive = a
                .folders
                .iter()
                .find(|f| f.role == Role::Archive)
                .unwrap()
                .id;
            let held = a.held.as_ref().unwrap();
            assert!(matches!(
                held.jobs[..],
                [(_, Job::Move { folder_id, ref uids, dest })]
                    if folder_id == a.folders[0].id && uids[..] == [5] && dest == archive
            ));
        });
        assert_eq!(unread(), 4);
        // A reload (a sync) doesn't bring it back meanwhile.
        h.app(|a| a.reload_rows());
        assert!(!listed(ids[1]));

        // Undo: back in the list, open again; no job is left.
        ui.invoke_undo_clicked();
        h.frame();
        assert!(listed(ids[1]));
        assert_eq!(open(), Some(ids[1]));
        assert!(h.app(|a| a.held.is_none()));
        assert_eq!(h.undo(), "");
        assert_eq!(unread(), 4);

        // A selection of three, deleted: one label, one held batch.
        ui.invoke_message_extended(ids[3] as i32);
        h.frame();
        ui.invoke_delete();
        h.frame();
        assert_eq!(h.undo(), "3 messages moved to Trash");
        let mut three = vec![ids[1], ids[2], ids[3]];
        three.sort_unstable();
        assert_eq!(held_ids(&h), three);
        // Ctrl+Z in the list undoes too: the three come back selected,
        // none opened.
        ui.invoke_focus_list();
        h.keys(&[Key::Control], "z");
        assert!(held_ids(&h).is_empty());
        assert_eq!(ui.get_selected_count(), 3);
        assert!(!ui.get_has_message());
        assert!(!h.app(|a| a.db.message(ids[3]).unwrap().unwrap().seen));

        // A second move sends the first one: only the last can be undone.
        ui.invoke_message_selected(ids[0] as i32);
        h.frame();
        ui.invoke_archive();
        h.frame();
        ui.invoke_delete();
        h.frame();
        assert!(!cached(ids[0]), "the archive went to the server");
        assert_eq!(held_ids(&h), [ids[1]]);
        // The undo window ends: the job goes and the offer with it.
        let seq = h.app(|a| a.held.as_ref().unwrap().seq);
        h.app(|a| a.flush_held());
        assert!(!cached(ids[1]));
        assert_eq!(h.undo(), "");
        // Undo after that does nothing.
        h.app(|a| a.undo_held(seq));
        assert!(!listed(ids[1]));

        // Closing the window sends what is held.
        ui.invoke_message_selected(ids[4] as i32);
        h.frame();
        ui.invoke_archive();
        h.frame();
        assert!(cached(ids[4]));
        ui.window().dispatch_event(WindowEvent::CloseRequested);
        assert!(!cached(ids[4]));
    }

    #[test]
    fn junk_moves_only_to_a_junk_folder() {
        let h = Harness::new("junk");
        let ids = more_messages(&h, &[]);
        let ui = h.ui();
        // No Junk folder: a clear line, and the message stays put.
        ui.invoke_message_selected(ids[0] as i32);
        h.frame();
        ui.invoke_junk(true);
        h.frame();
        assert_eq!(
            h.status(),
            "This account has no Junk folder, so nothing was moved. You can choose one in Settings: Accounts, Edit."
        );
        assert_eq!(h.ui().get_status_severity(), Severity::Critical);
        assert!(h.app(|a| a.held.is_none() && a.rows.index_of(ids[0]).is_some()));

        // With one: a held move there.
        let mut db = Db::open(&h.path).unwrap();
        let account = h.app(|a| a.accounts[0].id);
        let junk = db
            .upsert_folder(account, "Spam", "Spam", 0, Role::Junk)
            .unwrap();
        h.app(|a| a.load_accounts().unwrap());
        ui.invoke_junk(true);
        h.frame();
        assert_eq!(h.undo(), "Moved to Spam");
        h.app(|a| {
            assert!(matches!(
                a.held.as_ref().unwrap().jobs[..],
                [(_, Job::Move { dest, .. })] if dest == junk
            ));
            a.flush_held();
        });

        // In Junk: right-click an unread message; the menu offers Not junk
        // without opening it (or marking it read).
        let header = NewHeader {
            uid: 9,
            subject: "Prize".into(),
            date: 1_700_000_100,
            ..Default::default()
        };
        db.insert_headers(account, junk, &[header]).unwrap();
        h.app(|a| a.set_view(View::Folder(junk)));
        let spam = h.app(|a| a.rows.id_at(0)).unwrap();
        assert!(ui.invoke_message_context(spam as i32));
        h.frame();
        assert!(menu_has(ui, "Not junk") && menu_has(ui, "Mark as read"));
        assert!(!ui.get_has_message());
        assert_eq!(ui.get_selected_count(), 1);
        assert!(!h.app(|a| a.db.message(spam).unwrap().unwrap().seen));
        // Not junk: read now, moved to the Inbox after the undo window.
        ui.invoke_junk(false);
        h.frame();
        assert_eq!(h.undo(), "Moved to Inbox");
        h.app(|a| {
            assert!(a.db.message(spam).unwrap().unwrap().seen);
            let inbox = a.folders[0].id;
            assert!(matches!(
                a.held.as_ref().unwrap().jobs[..],
                [(_, Job::Move { folder_id, dest, .. })] if folder_id == junk && dest == inbox
            ));
        });
    }

    #[test]
    fn a_right_press_on_a_row_selects_it_without_opening_it() {
        use slint::platform::PointerEventButton;
        let h = Harness::new("right-press");
        let ids = more_messages(&h, &[]);
        let ui = h.ui();
        // The third row of the list (rows are 76 px under a header of
        // about 140 px).
        let position = slint::LogicalPosition::new(400.0, 140.0 + 76.0 * 2.5);
        let press = |button| {
            let w = ui.window();
            w.dispatch_event(WindowEvent::PointerPressed { position, button });
            w.dispatch_event(WindowEvent::PointerReleased { position, button });
            h.frame();
        };
        press(PointerEventButton::Right);
        assert_eq!(ui.get_selected_count(), 1);
        assert!(!ui.get_has_message(), "a right-press never opens a row");
        assert!(h.app(|a| a.rows.selection().contains(ids[2])));
        assert!(menu_has(ui, "Mark as read"));
    }

    #[test]
    fn right_click_selects_the_row_unless_it_is_selected() {
        let h = Harness::new("context");
        let ids = more_messages(&h, &[]);
        let ui = h.ui();
        let open = || h.app(|a| a.open.as_ref().map(|o| o.msg.id));
        // On the open message: it stays open and the menu acts on it.
        ui.invoke_message_selected(ids[0] as i32);
        h.frame();
        assert!(ui.invoke_message_context(ids[0] as i32));
        h.frame();
        assert_eq!(open(), Some(ids[0]));
        assert!(menu_has(ui, "Mark as unread"));
        // On another row: that row alone, not opened, not read.
        assert!(ui.invoke_message_context(ids[2] as i32));
        h.frame();
        assert_eq!(open(), None);
        assert_eq!(ui.get_selected_count(), 1);
        assert!(
            menu_has(ui, "Mark as read")
                && menu_has(ui, "Mark as important")
                && menu_has(ui, "Report junk")
        );
        assert!(!h.app(|a| a.db.message(ids[2]).unwrap().unwrap().seen));
        // On a row of a larger selection: the selection stays.
        ui.invoke_message_extended(ids[4] as i32);
        h.frame();
        assert!(ui.invoke_message_context(ids[3] as i32));
        h.frame();
        assert_eq!(ui.get_selected_count(), 3);
        // Reply needs a single message: nothing with three.
        ui.invoke_menu_respond(0);
        h.frame();
        assert!(!ui.get_composing());
        // Reply from the menu opens the one selected row, then replies
        // (the oldest message, the one with a body).
        assert!(ui.invoke_message_context(ids[5] as i32));
        ui.invoke_menu_respond(0);
        h.frame();
        assert!(ui.get_composing());
        assert_eq!(ui.get_c_subject(), "Re: Lunch");
        // No row menu while composing, nor in Drafts.
        assert!(!ui.invoke_message_context(ids[1] as i32));
        h.app(|a| a.set_view(View::Drafts));
        assert!(!ui.invoke_message_context(ids[1] as i32));
    }

    #[test]
    fn list_keys_work_from_the_start_and_wait_while_composing() {
        let h = Harness::new("list-keys");
        let ids = more_messages(&h, &[("Archive", Role::Archive)]);
        let ui = h.ui();
        let key = |k: Key| SharedString::from(k);
        let open = || h.app(|a| a.open.as_ref().map(|o| o.msg.id));
        let message = |id| h.app(|a| a.db.message(id).unwrap().unwrap());

        // No click first: the list has the keys from the start.
        h.keys(&[], &key(Key::DownArrow));
        assert_eq!(open(), Some(ids[0]));
        h.keys(&[], &key(Key::End));
        assert_eq!(open(), Some(ids[5]));
        h.keys(&[], &key(Key::Home));
        assert_eq!(open(), Some(ids[0]));
        // PageDown/PageUp move by the rows in view (all six fit here).
        h.keys(&[], &key(Key::PageDown));
        assert_eq!(open(), Some(ids[5]));
        h.keys(&[], &key(Key::PageUp));
        assert_eq!(open(), Some(ids[0]));

        // Esc closes the message; the arrows still work after it.
        h.keys(&[], &key(Key::Escape));
        assert_eq!(open(), None);
        assert_eq!(ui.get_selected_count(), 0);
        h.keys(&[], &key(Key::DownArrow));
        assert_eq!(open(), Some(ids[0]));
        // Esc with a search clears the search first, also typed into the
        // search box (where Insert doesn't reach the message, and Ctrl+E
        // only keeps the focus there).
        h.keys(&[Key::Control], "f");
        h.keys(&[], "m");
        h.keys(&[], "3");
        assert_eq!(ui.get_search_text(), "m3");
        assert_eq!(h.app(|a| a.rows.len()), 1);
        h.keys(&[], &key(Key::Insert));
        h.keys(&[Key::Control], "e");
        assert!(!message(ids[0]).flagged);
        assert!(h.app(|a| a.held.is_none()));
        h.keys(&[], &key(Key::Escape));
        assert_eq!(ui.get_search_text(), "");
        assert_eq!(h.app(|a| a.rows.len()), 6);
        assert_eq!(open(), Some(ids[0]));
        // Then the selection: two rows, Esc clears them.
        ui.invoke_message_selected(ids[0] as i32);
        ui.invoke_message_toggled(ids[2] as i32);
        h.frame();
        assert_eq!(ui.get_selected_count(), 2);
        h.keys(&[], &key(Key::Escape));
        assert_eq!(ui.get_selected_count(), 0);
        assert!(!ui.get_has_message());

        // Enter opens the current row (none: the first) and hands the
        // focus to the page.
        h.keys(&[], &key(Key::Return));
        assert_eq!(open(), Some(ids[0]));
        assert!(!ui.get_focus_page(), "the page took the focus");
        // Shift+Down from there, then Enter opens the cursor row.
        h.keys(&[Key::Shift], &key(Key::DownArrow));
        assert_eq!(open(), None);
        h.keys(&[], &key(Key::Return));
        assert_eq!(open(), Some(ids[1]));

        // Ctrl+U marks unread and Ctrl+Q read (Outlook; neither toggles),
        // Insert flags, Backspace archives.
        assert!(message(ids[1]).seen);
        h.keys(&[Key::Control], "u");
        assert!(!message(ids[1]).seen);
        h.keys(&[Key::Control], "u");
        assert!(!message(ids[1]).seen, "Ctrl+U is not a toggle");
        h.keys(&[Key::Control], "q");
        assert!(message(ids[1]).seen);
        h.keys(&[Key::Control], "q");
        assert!(message(ids[1]).seen, "Ctrl+Q is not a toggle");
        h.keys(&[], &key(Key::Insert));
        assert!(message(ids[1]).flagged);
        h.keys(&[], &key(Key::Backspace));
        assert_eq!(h.undo(), "Moved to Archive");
        assert_eq!(open(), Some(ids[2]));
        h.keys(&[Key::Control], "z");
        assert_eq!(open(), Some(ids[1]));

        // While composing, none of the list keys act on the message.
        ui.invoke_compose();
        h.frame();
        assert!(ui.get_composing());
        for k in [
            Key::DownArrow,
            Key::UpArrow,
            Key::Home,
            Key::End,
            Key::PageDown,
            Key::PageUp,
            Key::Return,
            Key::Insert,
        ] {
            h.keys(&[], &key(k));
        }
        h.keys(&[], &key(Key::Backspace));
        h.keys(&[Key::Control], "u");
        h.keys(&[Key::Control], "q");
        assert!(ui.get_composing());
        assert_eq!(open(), Some(ids[1]));
        let m = message(ids[1]);
        assert!(m.seen && m.flagged, "untouched");
        assert!(h.app(|a| a.held.is_none()));
        // Esc there closes the composer, not the message.
        h.keys(&[], &key(Key::Escape));
        assert!(!ui.get_composing());
        assert_eq!(open(), Some(ids[1]));
    }

    /// The title bar's search keys (docs/WINUI3.md "AutoSuggestBox") and
    /// Backspace = Archive with its scope (the keyboard map).
    #[test]
    fn search_keys_and_the_backspace_scope() {
        let h = Harness::new("search-keys");
        let ids = more_messages(&h, &[("Archive", Role::Archive)]);
        let ui = h.ui();
        let key = |k: Key| SharedString::from(k);
        let open = || h.app(|a| a.open.as_ref().map(|o| o.msg.id));

        // Ctrl+E, F3 and Ctrl+F each put the keys in the search box.
        let ctrl: &[Key] = &[Key::Control];
        for (mods, k) in [
            (ctrl, "e".into()),
            (&[][..], key(Key::F3)),
            (ctrl, "f".into()),
        ] {
            ui.invoke_focus_list();
            h.keys(mods, &k);
            h.keys(&[], "m");
            assert_eq!(ui.get_search_text(), "m", "{k:?}");
            ui.invoke_clear_search();
            h.frame();
        }
        // From Settings too: they close, and the mail view is searched.
        ui.invoke_focus_list();
        h.keys(&[Key::Control], ",");
        assert!(ui.get_show_settings());
        h.keys(&[Key::Control], "e");
        assert!(!ui.get_show_settings());
        h.keys(&[], "m");
        assert_eq!(ui.get_search_text(), "m");
        assert_eq!(ui.get_list_title(), "Search results");
        ui.invoke_clear_search();
        h.frame();
        // F6 from the list goes on round the panes: with nothing in the
        // reader, the search box is next (search, folders, list, reader).
        ui.invoke_focus_list();
        assert_eq!(open(), None);
        h.keys(&[], &key(Key::F6));
        h.keys(&[], "m");
        assert_eq!(ui.get_search_text(), "m");
        // Shift+F6 from the search box goes back to the list.
        h.keys(&[Key::Shift], &key(Key::F6));
        h.keys(&[], &key(Key::DownArrow));
        assert!(open().is_some(), "the list has the keys");
        ui.invoke_clear_search();
        h.frame();

        // Backspace in the search box edits the text: no archive.
        ui.invoke_message_selected(ids[0] as i32);
        h.frame();
        h.keys(&[Key::Control], "e");
        h.keys(&[], "m");
        h.keys(&[], &key(Key::Backspace));
        assert_eq!(ui.get_search_text(), "");
        assert!(h.app(|a| a.held.is_none()));
        // Esc in the search box clears the search ("Clear search (Esc)").
        h.keys(&[], "m");
        h.frame();
        assert_eq!(ui.get_list_title(), "Search results");
        h.keys(&[], &key(Key::Escape));
        h.frame();
        assert_eq!(ui.get_search_text(), "");
        assert_ne!(ui.get_list_title(), "Search results");
        // Nor in Drafts or the Outbox, whatever has the focus.
        for view in [View::Drafts, View::Outbox] {
            h.app(|a| a.set_view(view));
            ui.invoke_focus_list();
            h.keys(&[], &key(Key::DownArrow));
            h.keys(&[], &key(Key::Backspace));
            assert!(h.app(|a| a.held.is_none()), "{}", view.key());
        }
        // From a message row it archives the message.
        h.app(|a| a.set_view(View::Folder(a.folders[0].id)));
        ui.invoke_focus_list();
        h.keys(&[], &key(Key::DownArrow));
        assert_eq!(open(), Some(ids[0]));
        h.keys(&[], &key(Key::Backspace));
        assert_eq!(h.undo(), "Moved to Archive");
        assert!(h.app(|a| a.held.is_some()));
    }

    #[test]
    fn opened_messages_are_marked_read_after_the_delay() {
        let h = Harness::new("read-delay");
        let ids = more_messages(&h, &[]);
        let ui = h.ui();
        let seen = |id| h.app(|a| a.db.message(id).unwrap().unwrap().seen);
        h.app(|a| a.mark_read = MarkRead::After(Duration::from_secs(1)));

        // Arrowing past a message leaves it unread; only the one still
        // open when the delay ends is marked.
        ui.invoke_message_selected(ids[0] as i32);
        h.frame();
        assert!(!seen(ids[0]));
        assert!(ui.get_r_unread());
        assert!(h.app(|a| a.read_timer.running()));
        ui.invoke_select_offset(1);
        h.frame();
        h.app(|a| a.read_due());
        assert!(!seen(ids[0]));
        assert!(seen(ids[1]));
        assert!(!ui.get_r_unread());
        assert!(h.app(|a| a.rows.row_data(1).is_some_and(|r| !r.unread)));

        // Marked unread by hand meanwhile: the delay doesn't undo it.
        ui.invoke_select_offset(1);
        h.frame();
        ui.invoke_toggle_read();
        ui.invoke_toggle_read();
        h.frame();
        assert!(!seen(ids[2]));
        assert!(!h.app(|a| a.read_timer.running()));
        h.app(|a| a.read_due());
        assert!(!seen(ids[2]));

        // Closing the reader cancels the wait.
        ui.invoke_select_offset(1);
        h.frame();
        assert!(h.app(|a| a.read_timer.running()));
        ui.invoke_list_escape();
        h.frame();
        assert!(!h.app(|a| a.read_timer.running()));
        h.app(|a| a.read_due());
        assert!(!seen(ids[3]));

        // Never: opening leaves it unread; Now: read at once.
        h.app(|a| a.mark_read = MarkRead::Never);
        ui.invoke_message_selected(ids[4] as i32);
        h.frame();
        assert!(!seen(ids[4]) && !h.app(|a| a.read_timer.running()));
        h.app(|a| a.mark_read = MarkRead::Now);
        ui.invoke_message_selected(ids[3] as i32);
        h.frame();
        assert!(seen(ids[3]));
    }

    #[test]
    fn filters_empty_states_and_mark_all_read() {
        let h = Harness::new("filters");
        let ids = more_messages(&h, &[("Work", Role::Other)]);
        let ui = h.ui();
        let listed = || h.app(|a| a.rows.len());
        h.app(|a| {
            a.db.set_seen_many(&[ids[0], ids[1]], true).unwrap();
            a.db.set_flagged(ids[3], true).unwrap();
            a.reload_rows();
        });

        // Unread: the four unread; a message opened there stays listed
        // (read by now) until the filter or the view changes.
        ui.invoke_set_filter(1);
        h.frame();
        assert_eq!(ui.get_list_filter(), 1);
        assert_eq!(listed(), 4);
        ui.invoke_message_selected(ids[2] as i32);
        h.frame();
        assert_eq!(listed(), 4);
        h.app(|a| a.reload_rows());
        assert_eq!(listed(), 4);
        // Flagged, then a search with no match.
        ui.invoke_set_filter(2);
        h.frame();
        assert_eq!(listed(), 1);
        h.app(|a| a.db.set_flagged(ids[3], false).unwrap());
        ui.invoke_set_filter(2);
        h.frame();
        assert_eq!(listed(), 0);
        assert_eq!(ui.get_list_empty(), "No important messages");
        ui.set_search_text("nothing like it".into());
        ui.invoke_search_edited("nothing like it".into());
        h.frame();
        assert_eq!(ui.get_list_empty(), "No matches");
        ui.invoke_clear_search();
        h.frame();
        assert_eq!(ui.get_search_text(), "");

        // Mark all as read: every message of the view, at once.
        ui.invoke_set_filter(1);
        h.frame();
        assert_eq!(listed(), 3);
        ui.invoke_mark_all_read();
        h.frame();
        assert_eq!(listed(), 0);
        assert_eq!(ui.get_list_empty(), "No unread messages");
        assert!(h.app(|a| a.db.briefs(&ids).unwrap()).iter().all(|m| m.seen));
        assert_eq!(h.app(|a| a.nav.row_data(0).unwrap().unread), 0);
        assert!(!ui.get_r_unread());

        // Another folder: the filter is off again, and an empty folder
        // says whether its last sync failed.
        let account = h.app(|a| {
            let work = a.folders.iter().find(|f| f.display == "Work").unwrap().id;
            a.set_view(View::Folder(work));
            a.accounts[0].id
        });
        assert_eq!(ui.get_list_filter(), 0);
        assert_eq!(ui.get_list_empty(), "Nothing here");
        h.app(|a| {
            a.account_state.insert(account, AccountState::Unreachable);
            a.reload_rows();
        });
        assert!(ui.get_list_empty().starts_with("Can't reach the server"));
        // Drafts have no filters.
        h.app(|a| a.set_view(View::Drafts));
        ui.invoke_set_filter(1);
        h.frame();
        assert_eq!(ui.get_list_filter(), 0);
    }

    /// A server out of reach: Try now on the status line, and a caution
    /// badge on the account's pane row, rail tile and card that stays
    /// (also when the line is closed) until a sync goes through.
    #[test]
    fn the_offline_badge_stays_until_a_sync_goes_through() {
        use slint::Model;
        let h = Harness::new("offline-badge");
        let ui = h.ui();
        let account = h.app(|a| a.accounts[0].id);
        let offline = || {
            let e = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::TimedOut));
            Event::Syncing {
                account_id: account,
                active: false,
                error: Some(Problem::of(&e)),
            }
        };
        let synced = Event::Syncing {
            account_id: account,
            active: false,
            error: None,
        };
        let head = |ui: &AppWindow| {
            ui.get_nav_items()
                .iter()
                .find(|r| r.kind == 1 && i64::from(r.id) == account)
                .expect("the account's pane row")
        };
        let tile = |ui: &AppWindow| {
            ui.get_rail_items()
                .iter()
                .find(|r| i64::from(r.account) == account && !r.initial.is_empty())
                .expect("the account's rail tile")
        };
        assert!(!head(ui).caution && !tile(ui).caution);

        h.app(|a| a.on_event(offline()));
        assert_eq!(ui.get_status_action(), "Try now");
        assert!(h.app(|a| a.status_action.get() == StatusAction::TryNow(account)));
        let row = head(ui);
        assert!(row.caution && !row.warn);
        assert!(row.note.starts_with("Can't reach "), "{}", row.note);
        assert!(tile(ui).caution);
        assert!(ui.get_accounts().row_data(0).unwrap().caution);

        // Closing the line keeps the badge: the server is still out of reach.
        ui.invoke_dismiss_status();
        assert_eq!(ui.get_status(), "");
        assert!(head(ui).caution);

        // Try now: one attempt; the badge waits for its answer.
        h.app(|a| a.on_event(offline()));
        h.app(|a| a.set_status("", false));
        h.app(|a| a.try_now(account));
        assert!(head(ui).caution);

        // A sync that goes through clears it everywhere.
        h.app(|a| a.on_event(synced));
        assert!(!head(ui).caution && !tile(ui).caution);
        assert_eq!(head(ui).note, "");
        assert!(!ui.get_accounts().row_data(0).unwrap().caution);
    }

    /// A changed certificate: one short sentence and Review certificate
    /// on the status line; its card in Settings has the whole sentence
    /// and the same fix.
    #[test]
    fn a_changed_certificate_offers_review_certificate() {
        use slint::Model;
        let h = Harness::new("cert-changed");
        let ui = h.ui();
        let account = h.app(|a| a.accounts[0].id);
        crate::app::sample_cert_changed();
        assert_eq!(ui.get_status_action(), "Review certificate");
        assert!(h.app(|a| a.status_action.get() == StatusAction::ReviewCertificate(account)));
        assert!(
            ui.get_status()
                .ends_with("changed. Review it before you connect again."),
            "{}",
            ui.get_status()
        );
        let card = ui.get_accounts().row_data(0).unwrap();
        assert!(card.warn && card.review);
        assert!(
            card.status.contains("If you didn't expect that"),
            "{}",
            card.status
        );
    }

    /// A problem shows once per account until a sync goes through, and
    /// a folder dropped from the server's list leaves its view for the
    /// account's inbox.
    #[test]
    fn problems_show_once_and_a_vanished_folder_leads_to_the_inbox() {
        let h = Harness::new("resilience");
        more_messages(&h, &[("Work", Role::Other)]);
        let ui = h.ui();
        let (account, inbox, work) = h.app(|a| {
            let id = |name: &str| a.folders.iter().find(|f| f.display == name).unwrap().id;
            (a.accounts[0].id, id("Inbox"), id("Work"))
        });
        let offline = || {
            let e = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::TimedOut));
            Event::Syncing {
                account_id: account,
                active: false,
                error: Some(Problem::of(&e)),
            }
        };
        let synced = Event::Syncing {
            account_id: account,
            active: false,
            error: None,
        };
        h.app(|a| a.on_event(offline()));
        assert!(
            ui.get_status().contains("Can't reach the server"),
            "{}",
            ui.get_status()
        );
        // A Caution that stays: another line replaces it for a while, and
        // when that one goes, it is back.
        assert_eq!(ui.get_status_severity(), Severity::Caution);
        assert!(h.app(|a| !a.status_timer.running()));
        h.app(|a| a.set_status("Draft saved", false));
        assert_eq!(ui.get_status_severity(), Severity::Info);
        h.app(|a| a.set_status("", false));
        assert!(ui.get_status().contains("Can't reach the server"));
        // A sync that goes through takes it away.
        h.app(|a| a.on_event(synced.clone()));
        assert_eq!(ui.get_status(), "");
        h.app(|a| a.on_event(offline()));
        assert!(ui.get_status().contains("Can't reach the server"));
        ui.invoke_dismiss_status();
        // Closed: quiet, also when the line it waited under goes. After a
        // sync that went through, it shows again.
        h.app(|a| a.on_event(offline()));
        assert_eq!(ui.get_status(), "");
        h.app(|a| a.set_status("", false));
        assert_eq!(ui.get_status(), "");
        h.app(|a| {
            a.on_event(synced);
            a.on_event(offline());
        });
        assert!(ui.get_status().contains("Can't reach the server"));

        h.app(|a| a.set_view(View::Folder(work)));
        Db::open(&h.path)
            .unwrap()
            .retain_folders(account, &[inbox])
            .unwrap();
        h.app(|a| a.on_event(Event::FolderChanged { folder_id: None }));
        assert!(h.app(|a| a.view == View::Folder(inbox)));
        assert_eq!(ui.get_selected_folder(), inbox as i32);
    }

    #[test]
    fn search_all_folders() {
        let h = Harness::new("search-all");
        more_messages(&h, &[("Archive", Role::Archive), ("Sent", Role::Sent)]);
        let ui = h.ui();
        let (archive, sent) = h.app(|a| {
            let id = |name: &str| a.folders.iter().find(|f| f.display == name).unwrap().id;
            (id("Archive"), id("Sent"))
        });
        let mut db = Db::open(&h.path).unwrap();
        let account = h.app(|a| a.accounts[0].id);
        let joao = NewHeader {
            uid: 1,
            subject: "Orçamento".into(),
            from_name: "João Gonçalves".into(),
            from_addr: "joao@example.org".into(),
            date: 1_600_000_000,
            ..Default::default()
        };
        db.insert_headers(account, archive, std::slice::from_ref(&joao))
            .unwrap();
        let reply = NewHeader {
            subject: "Re: Orçamento".into(),
            from_name: "Me".into(),
            from_addr: "me@example.org".into(),
            to: "João Gonçalves <joao@example.org>".into(),
            date: 1_600_000_100,
            ..joao
        };
        db.insert_headers(account, sent, &[reply]).unwrap();
        let rows = || {
            h.app(|a| {
                (0..a.rows.len())
                    .map(|i| a.rows.row_data(i).unwrap())
                    .map(|r| (r.sender.to_string(), r.tag.to_string()))
                    .collect::<Vec<_>>()
            })
        };

        let inbox = h.app(|a| match a.view {
            View::Folder(id) => id,
            _ => panic!("one account opens on its Inbox"),
        });
        let account_name = h.app(|a| match a.accounts[0].name.trim() {
            "" => a.accounts[0].email.clone(),
            name => name.to_string(),
        });
        let tag = |folder: &str| format!("{account_name} · {folder}");

        // In the inbox, "joao" finds nothing; all folders find both, each
        // tagged with its account and folder, the sent one by recipient.
        // The title says "Search results"; the box's placeholder keeps the
        // view's name.
        ui.set_search_text("joao".into());
        ui.invoke_search_edited("joao".into());
        h.frame();
        assert!(rows().is_empty());
        assert_eq!(ui.get_list_empty(), "No matches");
        assert_eq!(ui.get_list_title(), "Search results");
        assert_eq!(ui.get_search_view(), "Inbox");
        assert_eq!(ui.get_list_subtitle(), "");
        assert_eq!(ui.get_result_count(), 0);
        ui.invoke_set_search_all(true);
        h.frame();
        assert!(ui.get_search_all());
        assert_eq!(ui.get_list_title(), "Search results");
        assert_eq!(ui.get_result_count(), 2);
        assert_eq!(
            rows(),
            [
                ("To: João Gonçalves <joao@example.org>".into(), tag("Sent")),
                ("João Gonçalves".into(), tag("Archive")),
            ]
        );
        // Typing on keeps the scope; folder actions keep to the view, so
        // the list's "..." menu offers no Mark all as read over results.
        ui.set_search_text("JOÃO GONÇ".into());
        ui.invoke_search_edited("JOÃO GONÇ".into());
        h.frame();
        assert_eq!(rows().len(), 2);
        assert!(
            !h.app(|a| a.list_menu()[0].enabled),
            "no Mark all as read while a search is showing"
        );
        ui.invoke_mark_all_read();
        h.frame();
        let archived = h.app(|a| a.db.message_ids(&[archive], "").unwrap());
        assert!(
            h.app(|a| a.db.briefs(&archived).unwrap())
                .iter()
                .all(|m| !m.seen)
        );
        // Another folder keeps the query and runs it there; only the scope
        // goes back to the view.
        h.app(|a| a.set_view(View::Folder(archive)));
        h.frame();
        assert!(!ui.get_search_all());
        assert_eq!(ui.get_search_text(), "JOÃO GONÇ");
        assert_eq!(rows(), [("João Gonçalves".into(), String::new())]);
        assert_eq!(ui.get_search_view(), "Archive");
        assert_eq!(ui.get_list_title(), "Search results");
        h.app(|a| a.set_view(View::Folder(inbox)));
        h.frame();
        assert!(rows().is_empty());
        // Cleared, the view lists as before, untagged.
        ui.invoke_clear_search();
        h.frame();
        // (One account: the view is its Inbox.)
        assert_eq!(ui.get_list_title(), "Inbox");
        assert_eq!(rows().len(), 6);
        assert!(rows().iter().all(|(_, folder)| folder.is_empty()));
        // Drafts have no scope.
        h.app(|a| a.set_view(View::Drafts));
        ui.invoke_set_search_all(true);
        h.frame();
        assert!(!ui.get_search_all());

        // Show in folder (the row menu while searching): the search clears
        // and the message opens in its own folder.
        h.app(|a| a.set_view(View::Folder(inbox)));
        ui.set_search_text("joao".into());
        ui.invoke_search_edited("joao".into());
        ui.invoke_set_search_all(true);
        h.frame();
        let found = h.app(|a| a.rows.id_at(1).unwrap());
        h.app(|a| a.select_message(found));
        ui.invoke_show_in_folder();
        h.frame();
        assert_eq!(ui.get_search_text(), "");
        assert!(!ui.get_search_all());
        assert!(h.app(|a| a.view == View::Folder(archive)));
        assert_eq!(h.app(|a| a.open.as_ref().map(|o| o.msg.id)), Some(found));
        assert_eq!(ui.get_list_title(), "Archive");
    }

    /// The sender notice, the encrypted state and Unsubscribe in the reader
    /// (Stage 16): computed on open, reset for the next message, and the
    /// two unsubscribe paths (composer, confirm prompt), with nothing sent.
    #[test]
    fn notice_encryption_and_unsubscribe_in_the_reader() {
        let h = Harness::new("notice");
        let id = h.message;
        let open = |raw: &[u8]| {
            Db::open(&h.path)
                .unwrap()
                .set_body(id, raw, "", false, Default::default())
                .unwrap();
            h.app(|a| {
                a.open = None;
                a.select_message(id);
            });
        };
        // The account's server (imap.example.invalid) failed DMARC, and the
        // list offers https and mailto: the mailto one is used.
        open(
            b"Authentication-Results: mx.example.invalid; spf=pass; dmarc=fail\r\n\
              Authentication-Results: mx.example.invalid; dmarc=pass\r\n\
              From: Bob <bob@example.org>\r\nTo: me@example.org\r\nSubject: Lunch\r\n\
              List-Unsubscribe: <https://list.example.org/u/1>,\r\n \
              <mailto:leave@list.example.org?subject=leave%20me>\r\n\r\nNoon?\r\n",
        );
        let ui = h.ui();
        assert_eq!(
            ui.get_r_notice(),
            "Reason: it failed your server's sender check (DMARC)."
        );
        assert!(ui.get_r_crypt().is_empty() && ui.get_r_unsubscribe());
        ui.invoke_unsubscribe();
        h.frame();
        assert!(ui.get_composing());
        assert_eq!(ui.get_c_to(), "leave@list.example.org");
        assert_eq!(ui.get_c_subject(), "leave me");
        assert!(h.app(|a| a.workers.is_empty()));
        ui.invoke_c_close();
        h.frame();

        // https only (plain http is never used): the confirm prompt.
        open(
            b"From: Shop <offers@shop.example.org>\r\nTo: me@example.org\r\nSubject: Sale\r\n\
              List-Unsubscribe: <http://shop.example.org/u>, <https://shop.example.org/u?x=1>\r\n\
              \r\nSale\r\n",
        );
        assert_eq!(ui.get_r_notice(), "");
        assert!(ui.get_r_unsubscribe());
        ui.invoke_unsubscribe();
        h.frame();
        assert!(ui.get_l_show());
        assert_eq!(ui.get_l_host(), "shop.example.org");
        assert_eq!(ui.get_l_after(), "/u?x=1");
        ui.invoke_l_cancel();
        h.frame();
        assert!(!ui.get_l_show() && !ui.get_composing());

        // Encrypted: the notice, the file to save, and no Unsubscribe left
        // over from the previous message.
        open(
            b"From: Bob <bob@example.org>\r\nTo: me@example.org\r\nSubject: Secret\r\n\
              MIME-Version: 1.0\r\nContent-Type: application/pkcs7-mime; smime-type=enveloped-data\r\n\
              Content-Transfer-Encoding: base64\r\n\r\nU3ludGhldGlj\r\n",
        );
        assert!(
            ui.get_r_crypt()
                .starts_with("This message is encrypted (S/MIME). Komorebi cannot decrypt it;")
        );
        assert!(ui.get_r_notice().is_empty() && !ui.get_r_unsubscribe());
        assert_eq!(ui.get_r_body(), "");
        let atts = ui.get_r_attachments();
        assert_eq!(atts.row_count(), 1);
        assert_eq!(atts.row_data(0).unwrap().name, "smime.p7m");
        ui.invoke_unsubscribe();
        h.frame();
        assert!(!ui.get_l_show() && !ui.get_composing());
    }

    /// The invite card (Stage 17): what it offers, the prompt naming the
    /// one recipient, the reply queued in the outbox (nothing is sent: the
    /// harness runs no worker), the answer kept, Change, an answer kept
    /// without a reply, and a cancelled invite.
    #[test]
    fn invite_card_answers_and_replies() {
        let h = Harness::new("invite");
        let id = h.message;
        let open = |raw: String| {
            Db::open(&h.path)
                .unwrap()
                .set_body(id, raw.as_bytes(), "", false, Default::default())
                .unwrap();
            h.app(|a| {
                a.open = None;
                a.select_message(id);
            });
        };
        // The list row's sender is bob@example.org (an organizer in that
        // domain raises no warning), and its To is the account.
        let invite = |method: &str, organizer: &str, attendees: &str, extra: &str| {
            format!(
                "From: Ana <ana@example.org>\r\nTo: me@example.org\r\nSubject: Plan\r\n\
                 Content-Type: text/calendar; method={method}\r\n\r\nBEGIN:VCALENDAR\r\n\
                 METHOD:{method}\r\nBEGIN:VEVENT\r\nUID:plan@example.org\r\nSEQUENCE:1\r\n\
                 DTSTART:20991008T130000Z\r\nSUMMARY:Planning\r\n\
                 DESCRIPTION:Agenda\r\nORGANIZER;CN=Ana:mailto:{organizer}\r\n{attendees}{extra}\
                 END:VEVENT\r\nEND:VCALENDAR\r\n"
            )
        };
        let both = "ATTENDEE;RSVP=TRUE:mailto:me@example.org\r\n\
                    ATTENDEE;RSVP=TRUE:mailto:carol@example.com\r\n";
        let ints = |m: ModelRc<i32>| m.iter().collect::<Vec<_>>();
        let account = h.app(|a| a.accounts[0].id);
        let ui = h.ui();

        open(invite("REQUEST", "ana@example.org", both, ""));
        assert_eq!(ui.get_i_title(), "Planning");
        assert_eq!(ui.get_i_state(), "Updated invitation");
        assert!(
            ui.get_i_lines()
                .contains("Organizer: Ana <ana@example.org>")
        );
        assert_eq!(ints(ui.get_i_answers()), [1, 2, 3]);
        // Details, then Save .ics.
        assert_eq!(ints(ui.get_i_tools()), [6, 4]);
        ui.invoke_invite(1);
        h.frame();
        assert_eq!(ui.get_q_title(), "Reply \u{201c}Accepted\u{201d}?");
        let text = ui.get_q_text();
        assert!(
            text.contains("only to the organizer, Ana <ana@example.org>, from me@example.org."),
            "{text}"
        );
        assert!(!text.contains("Check the address") && !text.contains("not listed"));
        assert_eq!(ui.get_q_alt(), "Don't send a reply");
        ui.invoke_q_answer(true);
        h.frame();
        let db = Db::open(&h.path).unwrap();
        let due = db.outbox_due(i64::MAX).unwrap();
        assert_eq!(due.len(), 1);
        let e = db.outbox_entry(due[0].0).unwrap().unwrap();
        assert_eq!(
            (e.to.as_str(), e.subject.as_str()),
            ("Ana <ana@example.org>", "Accepted: Planning")
        );
        assert!(e.invite_reply);
        assert_eq!(
            db.rsvp(account, "plan@example.org", "").unwrap(),
            Some((1, 1))
        );
        assert_eq!(ui.get_i_state(), "You accepted.");
        assert!(ints(ui.get_i_answers()).is_empty());
        assert_eq!(ints(ui.get_i_tools()), [5, 6, 4]);
        assert!(h.app(|a| a.workers.is_empty()));

        // Change, then Decline without a reply: kept here, nothing queued.
        ui.invoke_invite(5);
        h.frame();
        // The answer given is the accent button.
        assert_eq!(ints(ui.get_i_answers()), [1, 2, 3]);
        assert_eq!(ui.get_i_answer(), 1);
        ui.invoke_invite(3);
        h.frame();
        ui.invoke_q_alt_answer();
        h.frame();
        assert_eq!(ui.get_q_title(), "");
        assert_eq!(db.outbox_due(i64::MAX).unwrap().len(), 1);
        assert_eq!(
            db.rsvp(account, "plan@example.org", "").unwrap(),
            Some((1, 3))
        );
        assert_eq!(ui.get_i_state(), "You declined.");

        // The reply in the outbox: one recipient, the organizer; the REPLY
        // names only the account's attendee.
        let out = db.claim_outbox(due[0].0).unwrap().unwrap();
        let env = mail::smtp::envelope("me@example.org", &out.to, &out.cc, &out.bcc).unwrap();
        assert_eq!(env.to().len(), 1);
        let ics = mime::calendar_file(&out.raw).unwrap();
        let reply = mail::ical::parse(&ics).unwrap();
        assert_eq!(reply.method, "REPLY");
        assert_eq!(reply.attendees.len(), 1);
        assert_eq!(reply.attendees[0].addr, "me@example.org");
        assert_eq!(reply.attendees[0].partstat, "ACCEPTED");

        // Not an attendee, and an organizer outside the sender's domain:
        // the prompt says both.
        let other = "ATTENDEE;RSVP=TRUE:mailto:carol@example.com\r\n";
        open(invite("REQUEST", "x@elsewhere.example", other, ""));
        ui.invoke_invite(2);
        h.frame();
        let text = ui.get_q_text();
        assert!(
            text.contains("organizer's domain (elsewhere.example)"),
            "{text}"
        );
        assert!(text.contains("not listed as an attendee"), "{text}");
        ui.invoke_q_answer(false);
        h.frame();
        assert_eq!(db.outbox_due(i64::MAX).unwrap().len(), 0);

        // Cancelled: no answers, struck title.
        open(invite(
            "CANCEL",
            "ana@example.org",
            both,
            "STATUS:CANCELLED\r\n",
        ));
        assert!(ui.get_i_cancelled());
        assert!(ints(ui.get_i_answers()).is_empty());
        assert!(ui.get_i_state().starts_with("Cancelled"));

        // A message without an invite hides the card.
        open("From: Bob <bob@example.org>\r\nSubject: Lunch\r\n\r\nNoon?\r\n".into());
        assert_eq!(ui.get_i_title(), "");
    }
    #[test]
    fn source_actions_use_the_cached_bytes() {
        let h = Harness::new("source");
        let id = h.message;
        let raw = h.app(|a| a.db.message(id).unwrap().unwrap().raw.unwrap());
        let ui = h.ui();
        // Nothing open or selected: nothing to act on.
        ui.invoke_show_original();
        ui.invoke_copy_message_id();
        h.frame();
        assert!(!ui.get_src_show());
        assert_eq!(h.status(), "");

        h.app(|a| a.select_message(id));
        // Save as .eml writes exactly the cached bytes.
        let (name, bytes) = h.app(|a| a.eml_export()).unwrap();
        assert_eq!(bytes, raw);
        assert!(
            name.starts_with("Lunch (") && name.ends_with(").eml"),
            "{name}"
        );

        // Show original (More) shows the header fields and the source; Esc
        // closes it.
        ui.invoke_more_command(triage::CMD_ORIGINAL);
        h.frame();
        assert!(ui.get_src_show());
        assert_eq!(ui.get_src_msgid(), "<m1@example.org>");
        assert!(
            ui.get_src_headers()
                .contains("Subject: Lunch\nMessage-ID: <m1@example.org>")
        );
        assert_eq!(ui.get_src_body(), "Noon?\n");
        // The list keys are off while it shows.
        h.keys(&[Key::Control], "r");
        assert!(!ui.get_composing());
        h.keys(&[], &SharedString::from(Key::Escape));
        assert!(!ui.get_src_show());
        // Ctrl+U marks unread (Outlook) and no longer shows the original.
        h.keys(&[Key::Control], "u");
        assert!(!ui.get_src_show());
        assert!(!h.app(|a| a.db.message(id).unwrap().unwrap().seen));

        ui.invoke_copy_message_id();
        h.frame();
        assert_eq!(h.status(), "Message-ID copied.");

        // Forward as attachment: the message whole, as message/rfc822.
        ui.invoke_forward_attached();
        h.frame();
        assert!(ui.get_composing());
        assert_eq!(ui.get_c_title(), "Forward");
        assert_eq!(ui.get_c_subject(), "Fwd: Lunch");
        h.app(|a| {
            let d = a.drafts.active().unwrap();
            assert_eq!(d.attachments.len(), 1);
            assert_eq!(d.attachments[0].file.filename, "Lunch.eml");
            assert_eq!(d.attachments[0].file.data, raw);
            assert_eq!(d.reply_to, None);
        });
    }

    #[test]
    fn source_actions_wait_for_the_body() {
        let h = Harness::new("source-wait");
        let mut db = Db::open(&h.path).unwrap();
        let (account, inbox) = h.app(|a| (a.accounts[0].id, a.folders[0].id));
        let header = NewHeader {
            uid: 2,
            subject: "Report".into(),
            date: 1_700_000_100,
            ..Default::default()
        };
        db.insert_headers(account, inbox, &[header]).unwrap();
        h.app(|a| a.reload_rows());
        let id = h.app(|a| a.rows.id_at(0)).unwrap();
        assert_ne!(id, h.message);
        // From the row menu: the selected row opens first. No worker here
        // (offline), so there is nothing to wait for.
        h.app(|a| a.select_rows(|s, _| s.right_click(id)));
        h.ui().invoke_forward_attached();
        h.frame();
        assert_eq!(h.status(), "This message isn't downloaded yet.");
        assert_eq!(h.app(|a| a.open.as_ref().map(|o| o.msg.id)), Some(id));

        // A failed download says so; one that arrives runs the action.
        h.app(|a| {
            a.raw_wait = Some((id, original::RawUse::Forward));
            a.on_event(Event::BodyFailed {
                message_id: id,
                error: Problem::of(&anyhow::anyhow!("timed out")),
            });
            assert_eq!(a.raw_wait, None);
        });
        assert_eq!(h.status(), "Couldn't download this message: timed out");
        let raw = b"Subject: Report\r\n\r\nQ3\r\n";
        db.set_body(id, raw, "Q3", false, Default::default())
            .unwrap();
        h.app(|a| {
            a.raw_wait = Some((id, original::RawUse::Forward));
            // Another message's body changes nothing.
            a.on_event(Event::BodyReady {
                message_id: h.message,
            });
            assert!(a.raw_wait.is_some());
            a.on_event(Event::BodyReady { message_id: id });
            assert_eq!(a.raw_wait, None);
        });
        assert!(h.ui().get_composing());
        assert_eq!(h.ui().get_c_subject(), "Fwd: Report");
        h.app(|a| {
            let d = a.drafts.active().unwrap();
            assert_eq!(d.attachments[0].file.filename, "Report.eml");
            assert_eq!(d.attachments[0].file.data, raw);
        });
    }

    #[test]
    fn switched_off_accounts_spawn_no_workers() {
        let h = Harness::new("enabled");
        let id = h.app(|a| a.accounts[0].id);
        let row = |h: &Harness| h.ui().get_accounts().row_data(0).unwrap();
        assert!(row(&h).enabled);
        // Off: none now, and none when the workers are started.
        h.ui().invoke_account_enabled(id as i32, false);
        h.frame();
        assert!(!row(&h).enabled);
        assert!(row(&h).status.starts_with("Off"));
        assert!(h.app(|a| a.start_workers().is_empty() && a.workers.is_empty()));
        assert!(!h.app(|a| a.db.accounts().unwrap()[0].enabled));
        // On again (in the cache only here, so no sync is asked for): its
        // workers start, waiting for their first job.
        h.app(|a| {
            a.db.set_account_enabled(id, true).unwrap();
            a.load_accounts().unwrap();
        });
        assert_eq!(h.app(|a| a.start_workers()), [id]);
        assert!(h.app(|a| a.workers.contains_key(&id)));
        assert!(h.app(|a| a.start_workers().is_empty()));
        // Off again: they go.
        h.ui().invoke_account_enabled(id as i32, false);
        h.frame();
        assert!(h.app(|a| a.workers.is_empty()));
    }

    #[test]
    fn download_again_keeps_the_account_drafts_and_settings() {
        let h = Harness::new("redownload");
        let id = h.app(|a| a.accounts[0].id);
        h.app(|a| a.db.set_setting("undo_send_secs", "10").unwrap());
        h.ui().invoke_compose();
        h.frame();
        type_into(h.ui(), ComposeField::Body, "Keep me");
        h.app(|a| a.save_all_drafts());
        h.ui().invoke_c_close();
        h.ui().invoke_message_selected(h.message as i32);
        h.frame();
        assert!(h.ui().get_has_message());

        // It asks first, and says the server is not touched.
        h.ui().invoke_redownload_account(id as i32);
        h.frame();
        assert_eq!(
            h.ui().get_q_title(),
            "Download the mail of me@example.org again?"
        );
        assert!(
            h.ui()
                .get_q_text()
                .contains("Mail on the server is not affected"),
            "{}",
            h.ui().get_q_text()
        );
        h.ui().invoke_q_answer(false);
        h.frame();
        assert_eq!(h.ui().get_rows().row_count(), 1);

        h.ui().invoke_redownload_account(id as i32);
        h.frame();
        h.ui().invoke_q_answer(true);
        h.frame();
        // The downloaded mail is gone (offline here: no worker to fetch it).
        assert_eq!(h.ui().get_rows().row_count(), 0);
        assert!(!h.ui().get_has_message());
        assert!(h.app(|a| a.db.message(h.message).unwrap().is_none()));
        assert_eq!(
            h.status(),
            "The mail of me@example.org downloads again at its next sync."
        );
        // The account, its folders, its draft and the settings stay.
        h.app(|a| {
            assert_eq!(a.accounts.len(), 1);
            assert_eq!(a.accounts[0].id, id);
            assert_eq!(a.folders.len(), 1);
            assert_eq!(a.db.count_drafts().unwrap(), 1);
            assert_eq!(a.db.settings().unwrap().undo_send_secs, 10);
            assert!(a.workers.is_empty());
        });
    }

    /// The folder pane's rows as (label, level, expanded, unread).
    fn nav_rows(h: &Harness) -> Vec<(String, i32, bool, i32)> {
        h.ui()
            .get_nav_items()
            .iter()
            .map(|r| (r.label.to_string(), r.level, r.expanded, r.unread))
            .collect()
    }

    fn nav_row(h: &Harness, label: &str) -> usize {
        h.ui()
            .get_nav_items()
            .iter()
            .position(|r| r.label == label)
            .unwrap_or_else(|| panic!("no row {label}"))
    }

    #[test]
    fn the_first_default_folds_are_written_after_the_first_frame() {
        let h = Harness::new("navfoldsinit");
        h.app(|a| {
            let account = a.accounts[0].id;
            a.db.upsert_folder(account, "Projects", "Projects", 0, Role::Other)
                .unwrap();
            a.load_accounts().unwrap();
            assert!(a.folds_unsaved, "held in memory, not written yet");
            assert!(a.folds_init.contains(&account));
            assert_eq!(a.db.settings().unwrap().nav_folds_init, "");
            a.flush_default_folds();
            assert!(!a.folds_unsaved);
            assert_eq!(
                tree::parse_ids(&a.db.settings().unwrap().nav_folds_init),
                a.folds_init
            );
        });
    }

    #[test]
    fn folders_fold_into_a_tree_that_is_remembered() {
        let h = Harness::new("navtree");
        let (projects, alpha, q1) = h.app(|a| {
            let account = a.accounts[0].id;
            let add = |name: &str, display: &str, depth| {
                a.db.upsert_folder(account, name, display, depth, Role::Other)
                    .unwrap()
            };
            let ids = (
                add("Projects", "Projects", 0),
                add("Projects/Alpha", "Alpha", 1),
                add("Projects/Alpha/Q1", "Q1", 2),
            );
            add("Projects-old", "Projects-old", 0);
            let mut db = Db::open(&a.db_path).unwrap();
            let unread = |uid| NewHeader {
                uid,
                msgid: format!("q{uid}@example.org"),
                date: 1_700_000_000,
                ..Default::default()
            };
            db.insert_headers(account, ids.2, &[unread(1), unread(2)])
                .unwrap();
            a.load_accounts().unwrap();
            ids
        });
        let tree: Vec<(String, i32)> = nav_rows(&h)
            .into_iter()
            .skip(2)
            .map(|(l, level, ..)| (l, level))
            .collect();
        assert_eq!(
            tree,
            [
                ("Inbox".to_string(), 0),
                ("Projects".to_string(), 0),
                ("Alpha".to_string(), 1),
                ("Q1".to_string(), 2),
                ("Projects-old".to_string(), 0),
            ]
        );
        // Open Q1, then fold Projects: the view stays, the pill goes to
        // Projects, which shows the unread below it.
        h.app(|a| a.set_view(View::Folder(q1)));
        assert_eq!(h.ui().get_nav_pill() as usize, nav_row(&h, "Q1"));
        let row = nav_row(&h, "Projects");
        let item = h.ui().get_nav_items().row_data(row).unwrap();
        assert!(item.has_children && item.expanded);
        h.ui().invoke_nav_toggle(item);
        h.frame();
        assert!(h.app(|a| a.view == View::Folder(q1)));
        assert_eq!(h.ui().get_selected_folder() as i64, q1);
        assert_eq!(h.ui().get_nav_pill() as usize, row);
        let rows = nav_rows(&h);
        assert!(!rows.iter().any(|r| r.0 == "Alpha" || r.0 == "Q1"));
        assert!(!rows[row].2);
        assert_eq!(rows[row].3, 2);
        // Saved, and read back at the next start.
        let saved = h.app(|a| a.db.settings().unwrap().nav_folded);
        assert_eq!(saved, format!("f{projects}"));
        assert_eq!(
            tree::parse_folds(&saved),
            HashSet::from([Fold::Folder(projects)])
        );

        // The keys: Right opens Projects, Right again goes to its first
        // child, Alpha; Left folds the open Alpha, and Left again goes up
        // to Projects.
        h.ui().set_nav_cursor(row as i32);
        h.keys(&[Key::Shift], &SharedString::from(Key::F6));
        h.ui().set_nav_cursor(row as i32);
        h.keys(&[], &SharedString::from(Key::RightArrow));
        assert!(nav_rows(&h)[row].2, "Right opens Projects");
        assert_eq!(h.ui().get_nav_cursor() as usize, row);
        h.keys(&[], &SharedString::from(Key::RightArrow));
        assert_eq!(h.ui().get_nav_cursor() as usize, nav_row(&h, "Alpha"));
        h.keys(&[], &SharedString::from(Key::LeftArrow));
        let alpha_row = nav_row(&h, "Alpha");
        assert!(!nav_rows(&h)[alpha_row].2, "Left folds Alpha");
        assert_eq!(h.ui().get_nav_cursor() as usize, alpha_row);
        h.keys(&[], &SharedString::from(Key::LeftArrow));
        assert_eq!(h.ui().get_nav_cursor() as usize, row, "Left goes up");
        assert_eq!(
            h.app(|a| a.folded.clone()),
            HashSet::from([Fold::Folder(alpha)])
        );
        // Folding the account line from a folder inside it puts the cursor
        // on the line; the view still stays.
        let inbox = nav_row(&h, "Inbox");
        h.ui().set_nav_cursor(inbox as i32);
        h.keys(&[], &SharedString::from(Key::LeftArrow));
        let line = h.ui().get_nav_cursor() as usize;
        assert_eq!(h.ui().get_nav_items().row_data(line).unwrap().kind, 1);
        h.keys(&[], &SharedString::from(Key::LeftArrow));
        assert_eq!(h.ui().get_nav_items().row_count(), line + 1);
        assert_eq!(h.ui().get_nav_pill() as usize, line);
        assert!(h.app(|a| a.view == View::Folder(q1)));
        // A folder that is gone loses its fold.
        let account = h.app(|a| a.accounts[0].id);
        h.app(|a| {
            let keep: Vec<i64> = a
                .folders
                .iter()
                .map(|f| f.id)
                .filter(|&id| id != alpha)
                .collect();
            a.db.retain_folders(account, &keep).unwrap();
            a.load_accounts().unwrap();
        });
        assert_eq!(
            h.app(|a| a.folded.clone()),
            HashSet::from([Fold::Account(account)])
        );
        assert_eq!(
            h.app(|a| a.db.settings().unwrap().nav_folded),
            format!("a{account}")
        );
    }

    #[test]
    fn the_folder_pane_menu_and_rail_act_on_their_row() {
        let h = Harness::new("navmenu");
        let (projects, news, work_inbox) = h.app(|a| {
            let account = a.accounts[0].id;
            let add = |name: &str, depth| {
                a.db.upsert_folder(
                    account,
                    name,
                    name.rsplit('/').next().unwrap(),
                    depth,
                    Role::Other,
                )
                .unwrap()
            };
            let projects = add("Projects", 0);
            add("Projects/Alpha", 1);
            let news = add("Newsletters", 0);
            let mut db = Db::open(&a.db_path).unwrap();
            let unread = |uid| NewHeader {
                uid,
                msgid: format!("n{uid}@example.org"),
                date: 1_700_000_000,
                ..Default::default()
            };
            db.insert_headers(account, news, &[unread(1), unread(2)])
                .unwrap();
            let server = |host: &str| Server {
                host: host.into(),
                port: 993,
                security: Security::Tls,
            };
            let work = db
                .add_account(&NewAccount {
                    imap_user: String::new(),
                    smtp_user: String::new(),
                    email: "work@example.org".into(),
                    name: String::new(),
                    imap: server("imap.example.invalid"),
                    smtp: server("smtp.example.invalid"),
                    save_sent: true,
                })
                .unwrap();
            let work_inbox = db
                .upsert_folder(work, "INBOX", "Inbox", 0, Role::Inbox)
                .unwrap();
            a.load_accounts().unwrap();
            (projects, news, work_inbox)
        });
        let unread = |id: i64| h.app(|a| a.shown_unread().get(&id).copied().unwrap_or(0));

        // The second account starts folded (accounts after the first).
        let work = h.app(|a| a.accounts[1].id);
        assert!(h.app(|a| a.folded.contains(&Fold::Account(work))));
        // The rail: All inboxes and Drafts, then one tile per account,
        // opening its Inbox; its pill on the open folder's account.
        let rail: Vec<(String, i32, String)> = h
            .ui()
            .get_rail_items()
            .iter()
            .map(|r| (r.label.to_string(), r.id, r.initial.to_string()))
            .collect();
        assert_eq!(rail.len(), 4);
        assert_eq!(rail[1].0, "Drafts");
        assert_eq!(
            rail[3],
            ("work@example.org".into(), work_inbox as i32, "W".into())
        );
        h.app(|a| a.set_view(View::Folder(news)));
        assert_eq!(h.ui().get_rail_pill(), 2);
        h.ui()
            .invoke_nav_clicked(h.ui().get_rail_items().row_data(3).unwrap());
        h.frame();
        assert!(h.app(|a| a.view == View::Folder(work_inbox)));
        assert_eq!(h.ui().get_rail_pill(), 3);
        // A rail tile's menu goes to its account's Inbox, Sent or Archive
        // (greyed where the account has none).
        let tile = h.ui().get_rail_items().row_data(2).unwrap();
        assert!(h.ui().invoke_nav_context(tile.clone()));
        let entries: Vec<(String, bool, i32)> = h
            .ui()
            .get_nav_menu_main()
            .iter()
            .map(|e| (e.title.to_string(), e.enabled, e.command))
            .collect();
        assert_eq!(
            entries,
            [
                ("Inbox".to_string(), true, NAV_OPEN),
                ("Sent".to_string(), false, NAV_OPEN),
                ("Archive".to_string(), false, NAV_OPEN),
            ]
        );
        let inbox_entry = h.ui().get_nav_menu_main().row_data(0).unwrap();
        h.ui()
            .invoke_nav_command(tile, inbox_entry.command, inbox_entry.id);
        h.frame();
        assert!(h.app(|a| a.view == View::Folder(i64::from(inbox_entry.id))));
        h.app(|a| a.set_view(View::AllInboxes));

        let menu = |h: &Harness| -> Vec<(String, bool)> {
            let ui = h.ui();
            ui.get_nav_menu_main()
                .iter()
                .chain(ui.get_nav_menu_extra().iter())
                .map(|e| (e.title.to_string(), e.enabled))
                .collect()
        };
        // Mark all as read on a folder: exactly that folder, whatever view
        // is open.
        let item = |label: &str| h.ui().get_nav_items().row_data(nav_row(&h, label)).unwrap();
        assert!(!h.ui().invoke_nav_context(NavItem {
            kind: 3,
            ..Default::default()
        }));
        assert!(h.ui().invoke_nav_context(item("Newsletters")));
        assert_eq!(
            menu(&h),
            [
                ("Mark all as read".to_string(), true),
                ("Collapse all".to_string(), true),
                ("Hide folders".to_string(), true),
            ]
        );
        let inbox = h.app(|a| a.folders.iter().find(|f| f.role == Role::Inbox).unwrap().id);
        assert_eq!(unread(inbox), 1);
        h.ui()
            .invoke_nav_command(item("Newsletters"), NAV_MARK_READ, 0);
        h.frame();
        assert_eq!(unread(news), 0);
        assert_eq!(unread(inbox), 1);
        assert!(h.app(|a| a.view == View::AllInboxes));
        assert!(h.ui().invoke_nav_context(item("Newsletters")));
        assert!(!menu(&h)[0].1, "nothing left to mark read");
        // No Undo for it, as in Windows Mail.
        assert!(h.app(|a| a.held.is_none()));
        assert_eq!(h.undo(), "");

        // The account line: Collapse all folders folds its parent folders
        // (the account stays open); then nothing is left to collapse.
        let me = item("Me");
        assert!(h.ui().invoke_nav_context(me.clone()));
        assert_eq!(
            menu(&h),
            [
                ("Mark all as read".to_string(), true),
                ("Collapse all folders".to_string(), true),
                ("Refresh".to_string(), true),
                ("Account settings".to_string(), true),
                ("Hide folders".to_string(), true),
            ]
        );
        h.ui().invoke_nav_command(me.clone(), NAV_COLLAPSE, 0);
        h.frame();
        assert!(
            h.app(|a| a.folded == HashSet::from([Fold::Folder(projects), Fold::Account(work)]))
        );
        assert!(!nav_rows(&h).iter().any(|r| r.0 == "Alpha"));
        assert!(h.ui().invoke_nav_context(me.clone()));
        assert!(!menu(&h)[1].1, "nothing left to collapse");
        // Mark all as read on the account: its Inbox.
        h.ui().invoke_nav_command(me.clone(), NAV_MARK_READ, 0);
        h.frame();
        assert_eq!(unread(inbox), 0);
        // A folded account line has nothing to collapse.
        let work_line = item("work@example.org");
        assert_eq!(work_line.kind, 1);
        assert!(h.ui().invoke_nav_context(work_line));
        assert!(!menu(&h)[1].1, "a folded account's folders are folded");
        // A server Drafts folder's rows are never shown unread, so there
        // is nothing to mark read in it, whatever the cache says.
        let drafts = h.app(|a| {
            let account = a.accounts[0].id;
            let mut db = Db::open(&a.db_path).unwrap();
            let drafts = db
                .upsert_folder(account, "Drafts", "Drafts", 0, Role::Drafts)
                .unwrap();
            db.insert_headers(
                account,
                drafts,
                &[NewHeader {
                    uid: 1,
                    msgid: "d1@example.org".into(),
                    date: 1_700_000_000,
                    ..Default::default()
                }],
            )
            .unwrap();
            a.load_accounts().unwrap();
            drafts
        });
        let drafts_row = h
            .ui()
            .get_nav_items()
            .iter()
            .find(|r| r.kind == 2 && i64::from(r.id) == drafts)
            .expect("the server Drafts row");
        assert!(h.ui().invoke_nav_context(drafts_row));
        assert!(
            !menu(&h)[0].1,
            "no Mark all as read on a server Drafts folder"
        );
        h.app(|a| a.set_view(View::Folder(drafts)));
        h.frame();
        assert!(!h.app(|a| a.list_menu()[0].enabled));

        // Shift+F10 in the folder list asks for the current row's menu.
        let empty = || ModelRc::new(VecModel::from(Vec::<MenuCommand>::new()));
        h.keys(&[Key::Shift], &SharedString::from(Key::F6));
        h.ui().set_nav_cursor(nav_row(&h, "Me") as i32);
        h.ui().set_nav_menu_extra(empty());
        h.keys(&[Key::Shift], &SharedString::from(Key::F10));
        assert_eq!(
            h.ui().get_nav_menu_extra().row_count(),
            3,
            "the account line's menu was asked for"
        );
        // Also with a message open, when the list's own Shift+F10 is on:
        // the folder list has the key first.
        h.ui().invoke_message_selected(h.message as i32);
        h.frame();
        assert!(h.ui().get_has_message());
        h.ui().invoke_focus_list();
        h.keys(&[Key::Shift], &SharedString::from(Key::F6));
        h.ui().set_nav_cursor(nav_row(&h, "Me") as i32);
        h.ui().set_nav_menu_extra(empty());
        h.keys(&[Key::Shift], &SharedString::from(Key::F10));
        assert_eq!(
            h.ui().get_nav_menu_extra().row_count(),
            3,
            "the pane's menu, not the row menu"
        );

        // Account settings: Settings on that account.
        h.ui().invoke_nav_command(me, NAV_SETTINGS, 0);
        h.frame();
        assert!(h.ui().get_show_settings());
        assert_eq!(h.ui().get_a_edit_id() as i64, h.app(|a| a.accounts[0].id));
    }

    #[test]
    fn mark_all_read_is_one_function_from_the_pane_and_the_list() {
        let h = Harness::new("read-all");
        let (inbox, n) = h.app(|a| {
            let account = a.accounts[0].id;
            let inbox = a.folders[0].id;
            let mut db = Db::open(&a.db_path).unwrap();
            let unread = |uid| NewHeader {
                uid,
                msgid: format!("r{uid}@example.org"),
                date: 1_700_000_000 + i64::from(uid),
                ..Default::default()
            };
            db.insert_headers(account, inbox, &[unread(2), unread(3)])
                .unwrap();
            a.reload_rows();
            (inbox, a.shown_unread().get(&inbox).copied().unwrap_or(0))
        });
        assert_eq!(n, 3);
        // The list's "..." menu, on the Inbox (one account: the start view).
        assert!(h.app(|a| a.view == View::Folder(inbox)));
        h.ui().invoke_list_menu_opening();
        let entry = h.ui().get_list_menu().row_data(0).unwrap();
        assert_eq!(
            (entry.title.as_str(), entry.enabled),
            ("Mark all as read", true)
        );
        h.ui().invoke_list_command(entry.command);
        h.frame();
        let unread = |h: &Harness| h.app(|a| a.shown_unread().get(&inbox).copied().unwrap_or(0));
        assert_eq!(unread(&h), 0);
        assert!(h.app(|a| a.held.is_none()), "no undo record");
        assert_eq!(h.undo(), "");
        h.ui().invoke_list_menu_opening();
        assert!(!h.ui().get_list_menu().row_data(0).unwrap().enabled);

        // The pane's row menu on the same folder runs the same function:
        // the cache rows go read the same way and nothing is held.
        h.app(|a| {
            a.db.set_seen(h.message, false).unwrap();
        });
        let row = h
            .ui()
            .get_nav_items()
            .iter()
            .find(|r| r.kind == 2 && i64::from(r.id) == inbox)
            .unwrap();
        h.ui().invoke_nav_command(row, NAV_MARK_READ, 0);
        h.frame();
        assert_eq!(unread(&h), 0);
        assert!(h.app(|a| a.held.is_none()));
    }

    #[test]
    fn the_title_counts_unread_inbox_mail() {
        let h = Harness::new("title");
        // The one message starts unread.
        assert_eq!(h.ui().get_window_title(), "Inbox (1) - Komorebi");
        h.ui().invoke_message_selected(h.message as i32);
        h.frame();
        assert_eq!(h.ui().get_window_title(), "Komorebi");
        h.ui().invoke_toggle_read();
        h.frame();
        assert_eq!(h.ui().get_window_title(), "Inbox (1) - Komorebi");
    }

    #[test]
    fn the_language_switches_live_and_is_saved() {
        use komorebi::i18n::{Lang, LangSetting};
        let h = Harness::new("language");
        let ui = h.ui();
        ui.set_show_settings(true);
        h.frame();
        assert_eq!(ui.get_g_language(), 1, "English, as the harness set it");
        ui.set_show_settings(false);
        h.frame();
        assert_eq!(ui.get_window_title(), "Inbox (1) - Komorebi");
        // Portugal's Portuguese: the title is set again at once.
        ui.invoke_general_changed(6, 2);
        h.frame();
        assert_eq!(ui.get_window_title(), "Caixa de entrada (1) - Komorebi");
        h.app(|a| {
            assert_eq!(
                (a.language, a.lang),
                (LangSetting::PortuguesePortugal, Lang::PtPt)
            );
            assert_eq!(
                a.db.settings().unwrap().language,
                LangSetting::PortuguesePortugal
            );
        });
        // A later unread change keeps the language.
        ui.invoke_message_selected(h.message as i32);
        h.frame();
        assert_eq!(ui.get_window_title(), "Komorebi");
        ui.invoke_toggle_read();
        h.frame();
        assert_eq!(ui.get_window_title(), "Caixa de entrada (1) - Komorebi");
        // Back to English.
        ui.invoke_general_changed(6, 1);
        h.frame();
        assert_eq!(ui.get_window_title(), "Inbox (1) - Komorebi");
        h.app(|a| assert_eq!(a.lang, Lang::En));
        // System resolves from Windows: either way it is one of the two.
        ui.invoke_general_changed(6, 0);
        h.frame();
        h.app(|a| {
            assert_eq!(a.language, LangSetting::System);
            assert_eq!(a.db.settings().unwrap().language, LangSetting::System);
        });
        // Shown so when Settings opens again.
        ui.set_show_settings(false);
        h.frame();
        ui.set_show_settings(true);
        h.frame();
        assert_eq!(ui.get_g_language(), 0);
    }

    #[test]
    fn what_rust_set_is_set_again_when_the_language_changes() {
        let h = Harness::new("language-repush");
        let ui = h.ui();
        let account = h.app(|a| a.accounts[0].id);
        Db::open(&h.path)
            .unwrap()
            .upsert_folder(account, "Sent Items", "Sent Items", 0, Role::Sent)
            .unwrap();
        h.app(|a| a.on_event(Event::FolderChanged { folder_id: None }));
        h.frame();
        let labels = || -> Vec<String> {
            use slint::Model;
            ui.get_nav_items()
                .iter()
                .map(|i| i.label.to_string())
                .collect()
        };
        // English keeps the server's own spelling of a folder...
        assert!(
            labels().contains(&"Sent Items".to_string()),
            "{:?}",
            labels()
        );
        // ...the other languages name the folder by its role (the words
        // are the catalog's: Windows' Portuguese "Itens enviados"), in the
        // folder list at once.
        ui.invoke_general_changed(6, 2);
        h.frame();
        assert!(
            !labels().contains(&"Sent Items".to_string()),
            "{:?}",
            labels()
        );
        assert!(
            labels().contains(&"Itens enviados".to_string()),
            "{:?}",
            labels()
        );
        // The list's title follows too.
        assert_eq!(ui.get_list_title(), "Caixa de entrada");
        ui.invoke_general_changed(6, 1);
        h.frame();
        assert!(
            labels().contains(&"Sent Items".to_string()),
            "{:?}",
            labels()
        );
        assert_eq!(ui.get_list_title(), "Inbox");
    }

    #[test]
    fn general_choices_apply_and_are_saved() {
        use crate::db::ThemeChoice;
        use slint::language::ColorScheme;
        let h = Harness::new("general");
        let ui = h.ui();
        ui.set_show_settings(true);
        h.frame();
        // The values in use show (the defaults; the harness marks read at
        // once).
        let shown = || -> Vec<i32> { ui.get_g_choices().iter().collect() };
        assert_eq!(shown(), [0, 0, 1, 0]);
        assert!(!ui.get_g_flash());
        for (which, value) in [(0, 2), (1, 3), (2, 0), (3, 3), (4, 1)] {
            ui.invoke_general_changed(which, value);
        }
        h.frame();
        assert_eq!(
            ui.global::<crate::Palette>().get_color_scheme(),
            ColorScheme::Dark
        );
        h.app(|a| {
            assert_eq!(a.theme, ThemeChoice::Dark);
            assert_eq!(a.check_interval, Duration::from_secs(900));
            assert_eq!(a.undo_delay, Duration::ZERO);
            assert_eq!(a.mark_read, MarkRead::Never);
            assert!(a.flash);
            let s = a.db.settings().unwrap();
            assert_eq!(
                (
                    s.theme,
                    s.check_interval_secs,
                    s.undo_send_secs,
                    s.mark_read
                ),
                (ThemeChoice::Dark, 900, 0, MarkRead::Never)
            );
            assert!(s.flash_taskbar);
        });
        // Shown so when Settings opens again.
        ui.set_show_settings(false);
        h.frame();
        ui.set_show_settings(true);
        h.frame();
        assert_eq!(shown(), [2, 3, 0, 3]);
        assert!(ui.get_g_flash());
        ui.invoke_general_changed(0, 1);
        ui.invoke_general_changed(3, 0);
        // Out of range: the nearest end.
        ui.invoke_general_changed(2, 99);
        h.frame();
        assert_eq!(
            ui.global::<crate::Palette>().get_color_scheme(),
            ColorScheme::Light
        );
        assert_eq!(h.app(|a| a.mark_read), MarkRead::Now);
        assert_eq!(h.app(|a| a.undo_delay), Duration::from_secs(30));
        // Windows going dark (or light) is followed only under System.
        let windows_dark = |dark: bool| {
            h.app(|a| {
                let mut system = a.system.clone();
                system.dark = dark;
                a.system_changed(system);
            })
        };
        windows_dark(true);
        assert_eq!(
            ui.global::<crate::Palette>().get_color_scheme(),
            ColorScheme::Light
        );
        ui.invoke_general_changed(0, 0);
        h.frame();
        assert_eq!(
            h.app(|a| a.db.settings().unwrap().theme),
            ThemeChoice::System
        );
        assert_eq!(
            ui.global::<crate::Palette>().get_color_scheme(),
            ColorScheme::Dark
        );
        windows_dark(false);
        assert_eq!(
            ui.global::<crate::Palette>().get_color_scheme(),
            ColorScheme::Light
        );
    }

    #[test]
    fn windows_appearance_settings_are_followed() {
        use komorebi::theme::{self, CONTRAST_PRESETS, SystemTheme};
        use slint::language::ColorScheme;
        let h = Harness::new("appearance");
        let ui = h.ui();
        let t = ui.global::<crate::Theme>();
        let rgb = |c: slint::Color| [c.red(), c.green(), c.blue()];
        let show = |system: SystemTheme| h.app(|a| a.system_changed(system));
        let green =
            theme::parse_colors::<7>("#0c1c14,#173525,#1e4732,#26593f,#2e6a4b,#357c58,#40956a")
                .unwrap();
        // The accent fill is Dark1 with white text in light, Light2 with
        // black text in dark; selected text is on the accent itself.
        for dark in [false, true] {
            show(SystemTheme {
                ramp: green,
                dark,
                ..SystemTheme::default()
            });
            assert_eq!(
                ui.global::<crate::Palette>().get_color_scheme() == ColorScheme::Dark,
                dark
            );
            assert_eq!(rgb(t.get_accent()), green[if dark { 5 } else { 2 }]);
            let on = if dark { [0, 0, 0] } else { [255, 255, 255] };
            assert_eq!(rgb(t.get_on_accent()), on);
            assert_eq!(rgb(t.get_selection_bg()), green[3]);
            assert!(!t.get_contrast());
            // WinUI's translucent values (#AARRGGBB in docs/WINUI3.md):
            // TextFillColorPrimary and SubtleFillColorSecondary.
            let argb = |c: slint::Color| (c.alpha(), rgb(c));
            let (text, hover) = if dark {
                ((0xff, [255; 3]), (0x0f, [255; 3]))
            } else {
                ((0xe4, [0; 3]), (0x09, [0; 3]))
            };
            assert_eq!(argb(t.get_text()), text);
            assert_eq!(argb(t.get_hover()), hover);
        }
        // A contrast theme: light or dark as its window colour is, whatever
        // Windows' app mode; the accent and focus take its colours.
        for (name, dark) in [("hcblack", true), ("hcwhite", false)] {
            let hc = CONTRAST_PRESETS.iter().find(|(n, _)| *n == name).unwrap().1;
            let hc = theme::parse_colors::<8>(hc).unwrap();
            show(SystemTheme {
                dark: !dark,
                contrast: Some(hc),
                ..SystemTheme::default()
            });
            assert!(t.get_contrast());
            assert_eq!(
                ui.global::<crate::Palette>().get_color_scheme() == ColorScheme::Dark,
                dark,
                "{name}"
            );
            assert_eq!(rgb(t.get_accent()), hc[2], "{name}");
            assert_eq!(rgb(t.get_on_accent()), hc[3], "{name}");
            assert_eq!(rgb(t.get_focus()), hc[1], "{name}");
            // Every colour takes the theme's: surfaces Window, text
            // WindowText, subtle fills Highlight under HighlightText, all
            // opaque.
            for c in [t.get_layer(), t.get_card(), t.get_info_bg()] {
                assert_eq!((rgb(c), c.alpha()), (hc[0], 255), "{name}");
            }
            for c in [t.get_text(), t.get_text_2(), t.get_text_3(), t.get_stroke()] {
                assert_eq!((rgb(c), c.alpha()), (hc[1], 255), "{name}");
            }
            for c in [t.get_hover(), t.get_pressed(), t.get_selected()] {
                assert_eq!((rgb(c), c.alpha()), (hc[2], 255), "{name}");
            }
            assert_eq!(rgb(t.get_on_selected()), hc[3], "{name}");
            assert_eq!(rgb(t.invoke_on_fill(true, t.get_text_2())), hc[3]);
            assert_eq!(rgb(t.get_text_disabled()), hc[5], "{name}");
        }
        // Text size: the type ramp and the rows grow, animations stop.
        show(SystemTheme {
            text_scale: 2.25,
            animate: false,
            ..SystemTheme::default()
        });
        assert!((t.get_body() - 27.5).abs() < 0.05, "{}", t.get_body());
        assert!((t.get_caption() - 26.1).abs() < 0.05);
        assert_eq!(t.get_row_height(), 131.0);
        assert_eq!(t.get_fast(), 0);
        show(SystemTheme::default());
        assert_eq!((t.get_body(), t.get_row_height()), (14.0, 76.0));
        assert_eq!(t.get_fast(), 167);
    }

    #[test]
    fn settings_open_with_ctrl_comma_and_escape_steps_back() {
        let h = Harness::new("settings-keys");
        let ui = h.ui();
        h.keys(&[Key::Control], ",");
        assert!(ui.get_show_settings());
        h.keys(&[], "\u{1b}");
        assert!(!ui.get_show_settings());
        // From the account form, Esc goes back to the list first.
        let id = h.app(|a| a.accounts[0].id);
        ui.invoke_edit_account(id as i32);
        h.frame();
        assert!(ui.get_show_settings());
        assert_eq!(ui.get_settings_section(), 0);
        h.keys(&[], "\u{1b}");
        assert!(ui.get_show_settings());
        assert_eq!(ui.get_a_edit_id(), 0);
        h.keys(&[], "\u{1b}");
        assert!(!ui.get_show_settings());
        // The list keys work again.
        h.keys(&[], &SharedString::from(Key::DownArrow));
        assert!(ui.get_has_message());
    }

    // ------------------------------------------- keyboard focus (Stage 21)

    fn key(k: Key) -> SharedString {
        SharedString::from(k)
    }

    #[test]
    fn tab_reaches_the_reader_toolbar_as_one_stop() {
        let h = Harness::new("tab-toolbar");
        let ui = h.ui();
        let id = h.message;
        let message = || h.app(|a| a.db.message(id).unwrap().unwrap());
        h.app(|a| a.select_message(id));
        assert!(ui.get_has_message());
        // From the list, Tab lands on Reply; Space presses it.
        ui.invoke_focus_list();
        h.keys(&[], &key(Key::Tab));
        h.keys(&[], " ");
        assert!(ui.get_composing());
        assert_eq!(ui.get_c_title(), "Reply");
        ui.invoke_c_close();
        h.frame();

        // Right moves along the toolbar, past Move (off: one folder only),
        // and Enter presses the button reached: Unread.
        ui.invoke_focus_list();
        h.keys(&[], &key(Key::Tab));
        assert_eq!(ui.get_move_targets().row_count(), 0);
        for _ in 0..5 {
            h.keys(&[], &key(Key::RightArrow));
        }
        h.keys(&[], &key(Key::Return));
        assert!(!message().seen);
        // The toolbar is one stop: Tab leaves it (Enter there flags
        // nothing), and Shift+Tab comes back to the button last reached.
        h.keys(&[], &key(Key::Tab));
        h.keys(&[], &key(Key::Return));
        assert!(!message().flagged && !message().seen);
        h.keys(&[Key::Shift], &key(Key::Tab));
        h.keys(&[], &key(Key::Return));
        assert!(message().seen);
        // Left skips Move too: four from Unread reach Reply all.
        assert!(!ui.get_composing());
        h.keys(&[], &key(Key::LeftArrow));
        h.keys(&[], &key(Key::LeftArrow));
        h.keys(&[], &key(Key::LeftArrow));
        h.keys(&[], &key(Key::LeftArrow));
        h.keys(&[], " ");
        assert_eq!(ui.get_c_title(), "Reply all");
    }

    #[test]
    fn the_formatting_row_is_one_tab_stop() {
        let h = Harness::new("tab-format");
        let ui = h.ui();
        // A plain-text message has no formatting row: To, its Cc and Bcc
        // links, Subject, Signature and More (the importance toggles are
        // in More at this width), then the body.
        ui.invoke_compose();
        h.frame();
        for _ in 0..6 {
            h.keys(&[], &key(Key::Tab));
        }
        h.keys(&[], "Z");
        assert_eq!(ui.get_c_body(), "Z");
        ui.invoke_c_close();
        h.frame();
        // Formatted: the buttons are one stop before Signature; from the
        // body, Shift+Tab past More and Signature reaches Bold, and Right
        // goes on to Italic.
        h.app(|a| a.compose_rich = true);
        h.app(|a| a.discard_confirmed());
        ui.invoke_compose();
        h.frame();
        assert!(ui.get_c_rich());
        h.app(|a| a.focus_body());
        h.frame();
        h.keys(&[], "Z");
        assert_eq!(rich_text(&h), "Z");
        for _ in 0..3 {
            h.keys(&[Key::Shift], &key(Key::Tab));
        }
        h.keys(&[], &key(Key::RightArrow));
        h.keys(&[], " ");
        let fmt = ui.global::<crate::Rich>().get_body_fmt();
        assert!(fmt.i && !fmt.b, "Italic is on for what is typed next");
    }

    #[test]
    fn f6_cycles_the_panes_and_the_folders_take_arrows() {
        let h = Harness::new("f6");
        let ui = h.ui();
        let id = h.message;
        h.app(|a| a.select_message(id));
        ui.invoke_focus_list();
        // F6: list -> reader (the toolbar's Reply).
        h.keys(&[], &key(Key::F6));
        h.keys(&[], " ");
        assert_eq!(ui.get_c_title(), "Reply");
        // From the composer, F6 goes to the folders; Shift+F6 comes back
        // into the composer (its To field takes the typing).
        h.keys(&[], &key(Key::F6));
        h.keys(&[Key::Shift], &key(Key::F6));
        h.keys(&[], "Q");
        assert!(ui.get_c_to().contains('Q'), "{}", ui.get_c_to());
        ui.invoke_c_close();
        h.frame();

        // In the folders, Up/Down move and Enter opens; the focus stays
        // there for the next one (from Drafts: one account starts on its
        // Inbox).
        h.app(|a| a.set_view(View::Drafts));
        let items = ui.get_nav_items();
        let folder = (0..items.row_count())
            .find(|&i| items.row_data(i).unwrap().kind == 2)
            .expect("a folder row");
        let inbox = items.row_data(folder).unwrap().id;
        assert_ne!(ui.get_selected_folder(), inbox);
        ui.invoke_focus_list();
        h.keys(&[Key::Shift], &key(Key::F6));
        h.keys(&[], &key(Key::Home));
        for _ in 0..folder {
            h.keys(&[], &key(Key::DownArrow));
        }
        h.keys(&[], &key(Key::Return));
        assert_eq!(ui.get_selected_folder(), inbox);
        h.keys(&[], &key(Key::Home));
        h.keys(&[], " ");
        // (The first row: Drafts.)
        assert_eq!(ui.get_selected_folder(), -1);
        // The arrows stayed in the folders: no message was opened.
        assert!(!ui.get_has_message());
        // Back on the Inbox, F6 to the list, where Down opens the first
        // message.
        for _ in 0..folder {
            h.keys(&[], &key(Key::DownArrow));
        }
        h.keys(&[], &key(Key::Return));
        h.keys(&[], &key(Key::F6));
        h.keys(&[], &key(Key::DownArrow));
        assert!(ui.get_has_message());
    }

    #[test]
    fn dialogs_keep_the_keyboard_and_start_on_a_safe_button() {
        let h = Harness::new("dialog-focus");
        let ui = h.ui();
        let account = h.app(|a| a.accounts[0].id);
        // The prompt starts on Cancel: Enter never confirms a removal.
        h.app(|a| a.ask(Question::RemoveAccount(account)));
        assert_ne!(ui.get_q_title(), "");
        assert!(ui.get_q_destructive());
        h.keys(&[], &key(Key::Return));
        assert_eq!(ui.get_q_title(), "");
        assert_eq!(h.app(|a| a.accounts.len()), 1);
        // The list has the keys again.
        h.keys(&[], &key(Key::DownArrow));
        assert!(ui.get_has_message());
        // Tab goes Cancel -> Remove -> back to Cancel, never behind it.
        h.app(|a| a.ask(Question::RemoveAccount(account)));
        h.keys(&[], &key(Key::Tab));
        h.keys(&[], &key(Key::Tab));
        h.keys(&[], " ");
        assert_eq!(ui.get_q_title(), "");
        assert_eq!(h.app(|a| a.accounts.len()), 1);
        // Over the composer, the discard prompt hands the body back.
        ui.invoke_compose();
        h.frame();
        ui.set_c_confirm_discard(true);
        h.frame();
        h.keys(&[], &key(Key::Escape));
        assert!(ui.get_composing() && !ui.get_c_confirm_discard());
        h.keys(&[], "Z");
        assert_eq!(ui.get_c_body(), "Z");
        ui.invoke_c_close();
        h.frame();

        // Settings (a page, not a dialog) start on the tabs, one Tab stop
        // whose arrows select; the list and reader under the page are no
        // Tab stops: past the General cards Tab reaches the taskbar flash
        // (with no status line, whose close button comes before them).
        ui.invoke_dismiss_status();
        ui.set_show_settings(true);
        h.frame();
        // Accounts, Signatures, then General.
        h.keys(&[], &key(Key::RightArrow));
        h.keys(&[], &key(Key::RightArrow));
        assert_eq!(ui.get_settings_section(), 2);
        assert!(!h.app(|a| a.flash));
        for _ in 0..5 {
            h.keys(&[], &key(Key::Tab));
        }
        h.keys(&[], " ");
        assert!(h.app(|a| a.flash));
        // Many Tabs go round the page and the folder pane; Esc still
        // closes Settings from wherever the focus is.
        for _ in 0..30 {
            h.keys(&[], &key(Key::Tab));
        }
        // A prompt over Settings gives the focus back to the button that
        // asked: Privacy > Remove all data (tabs, diagnostics, logs
        // folder, then it). A destructive question starts on Cancel.
        ui.set_settings_section(3);
        h.frame();
        ui.set_show_settings(false);
        h.frame();
        ui.set_show_settings(true);
        h.frame();
        for _ in 0..3 {
            h.keys(&[], &key(Key::Tab));
        }
        h.keys(&[], " ");
        assert_ne!(ui.get_q_title(), "");
        assert!(ui.get_q_destructive());
        h.keys(&[], &key(Key::Escape));
        assert_eq!(ui.get_q_title(), "");
        assert!(ui.get_show_settings());
        h.keys(&[], " ");
        assert_ne!(ui.get_q_title(), "", "the focus is back on Remove all data");
        h.keys(&[], &key(Key::Escape));
        h.keys(&[], &key(Key::Escape));
        assert!(!ui.get_show_settings());
    }

    #[test]
    fn settings_show_an_accounts_download() {
        use slint::Model;
        let h = Harness::new("settings-progress");
        let ui = h.ui();
        let account = h.app(|a| a.accounts[0].id);
        h.app(|a| {
            a.on_event(Event::Progress {
                account_id: account,
                folder_id: 0,
                done: 1234,
                total: 5000,
            })
        });
        let row = ui.get_accounts().row_data(0).unwrap();
        assert_eq!((row.done, row.total), (1234, 5000));
        assert_eq!(row.status, "Downloading 1,234 of 5,000\u{a0}…");
    }

    #[test]
    fn remove_all_data_forgets_every_password_then_closes_the_cache() {
        let h = Harness::new("remove-all");
        let ui = h.ui();
        let accounts = h.app(|a| a.accounts.iter().map(|x| x.id).collect::<Vec<_>>());
        // A fake credential store, unavailable at first.
        let forgotten = Rc::new(RefCell::new(Vec::new()));
        let available = Rc::new(Cell::new(false));
        let store = (forgotten.clone(), available.clone());
        h.app(|a| {
            a.forget_password = Rc::new(move |id| {
                anyhow::ensure!(store.1.get(), "{}", secrets::UNAVAILABLE);
                store.0.borrow_mut().push(id);
                Ok(())
            });
        });
        // It asks first; Cancel removes nothing.
        ui.invoke_remove_all_data();
        h.frame();
        assert_eq!(ui.get_q_title(), "Remove all of Komorebi's data?");
        ui.invoke_q_answer(false);
        h.frame();
        assert!(forgotten.borrow().is_empty());
        assert!(!h.app(|a| a.remove_data));
        // A password that can't be deleted stops it before anything else.
        ui.invoke_remove_all_data();
        ui.invoke_q_answer(true);
        h.frame();
        assert!(h.status().starts_with("Stopped, no files removed."));
        assert!(!h.app(|a| a.remove_data));
        assert_eq!(h.app(|a| a.accounts.len()), 1);
        assert!(h.path.exists());

        available.set(true);
        ui.invoke_remove_all_data();
        ui.invoke_q_answer(true);
        h.frame();
        assert_eq!(*forgotten.borrow(), accounts);
        assert!(h.app(|a| a.remove_data && a.workers.is_empty() && a.accounts.is_empty()));
        // After the window closes: the cache is closed, so every file goes.
        let mut h = h;
        let path = close_for_removal(h.ui.take().unwrap()).expect("removal asked");
        assert!(with_app(|_| ()).is_none());
        assert_eq!(
            wipe::remove_files_within(&path, Duration::ZERO),
            [] as [PathBuf; 0]
        );
        assert!(!path.exists());
        assert!(!db::drafts_dir(&path).exists());
    }

    // ------------------------------------------------ triage (UX Stage 3)

    /// The reader bar's labels go by priority, each measured on its own:
    /// pt-PT at 1280 x 800 keeps Reply, Archive and Delete, English also
    /// Move; narrower, Archive is the last to lose its label.
    #[test]
    fn reader_bar_labels_go_by_priority() {
        const MOVE: i32 = 4;
        const REPLY: i32 = 32;
        const DELETE: i32 = 64;
        const ARCHIVE: i32 = 128;
        let h = Harness::new("labels");
        // A second folder, so Move can run.
        more_messages(&h, &[("Archive", Role::Archive)]);
        let ui = h.ui();
        h.app(|a| a.select_message(h.message));
        assert!(ui.get_has_message() && ui.get_move_possible());
        let en = ui.get_cmd_labels();
        let want = REPLY | ARCHIVE | DELETE | MOVE;
        assert_eq!(en & want, want, "English at 1280: {en:#010b}");
        ui.invoke_general_changed(6, 2);
        h.frame();
        let pt = ui.get_cmd_labels();
        let want = REPLY | ARCHIVE | DELETE;
        assert_eq!(pt & want, want, "pt-PT at 1280: {pt:#010b}");
        // Narrower: labels go, the last kept is always Archive.
        for width in [1100, 1000, 900, 800, 720] {
            ui.window().set_size(slint::PhysicalSize::new(width, 800));
            h.frame();
            let got = ui.get_cmd_labels();
            assert!(got == 0 || got & ARCHIVE != 0, "{width}: {got:#010b}");
            assert!(
                got & DELETE == 0 || got & ARCHIVE != 0,
                "{width}: {got:#010b}"
            );
        }
        ui.window().set_size(slint::PhysicalSize::new(1280, 800));
        ui.invoke_general_changed(6, 1);
        h.frame();
    }

    /// A row's quick actions act on that row only: never opened, never
    /// marked read unless it is Mark as read, the open message and the
    /// selection unchanged.
    #[test]
    fn quick_actions_never_open_or_mark_their_row_read() {
        let h = Harness::new("quick");
        let ids = more_messages(&h, &[("Archive", Role::Archive), ("Trash", Role::Trash)]);
        let ui = h.ui();
        let message = |id| h.app(|a| a.db.message(id).unwrap().unwrap());
        let open = || h.app(|a| a.open.as_ref().map(|o| o.msg.id));
        ui.invoke_message_selected(ids[0] as i32);
        h.frame();
        assert!(!message(ids[2]).seen);
        // Flag and Mark as read / unread on another row.
        ui.invoke_message_quick(ids[2] as i32, triage::QUICK_FLAG);
        h.frame();
        assert!(message(ids[2]).flagged && !message(ids[2]).seen);
        ui.invoke_message_quick(ids[2] as i32, triage::QUICK_FLAG);
        h.frame();
        assert!(!message(ids[2]).flagged && !message(ids[2]).seen);
        ui.invoke_message_quick(ids[2] as i32, triage::QUICK_READ);
        h.frame();
        assert!(message(ids[2]).seen);
        ui.invoke_message_quick(ids[2] as i32, triage::QUICK_READ);
        h.frame();
        assert!(!message(ids[2]).seen);
        assert_eq!(open(), Some(ids[0]));
        assert_eq!(ui.get_selected_count(), 1);
        // Archive another row: held (with Undo), the open message stays.
        ui.invoke_message_quick(ids[3] as i32, triage::QUICK_ARCHIVE);
        h.frame();
        assert_eq!(held_ids(&h), [ids[3]]);
        assert_eq!(open(), Some(ids[0]));
        assert!(!message(ids[3]).seen, "not read by archiving");
        assert_eq!(h.undo(), "Moved to Archive");
        // Delete another row: to Trash, held.
        ui.invoke_message_quick(ids[4] as i32, triage::QUICK_DELETE);
        h.frame();
        assert_eq!(held_ids(&h), [ids[4]]);
        assert_eq!(open(), Some(ids[0]));
        assert_eq!(h.undo(), "Moved to Trash");
        // A selection of three: archiving one of them from its row leaves
        // the other two selected (nothing opens).
        ui.invoke_message_extended(ids[2] as i32);
        h.frame();
        assert_eq!((ui.get_selected_count(), open()), (3, None));
        ui.invoke_message_quick(ids[1] as i32, triage::QUICK_ARCHIVE);
        h.frame();
        assert_eq!(held_ids(&h), [ids[1]]);
        assert_eq!((ui.get_selected_count(), open()), (2, None));
        assert!(
            h.app(|a| a.rows.selection().contains(ids[0]) && a.rows.selection().contains(ids[2]))
        );
        // None of it while composing.
        ui.invoke_compose();
        h.frame();
        ui.invoke_message_quick(ids[2] as i32, triage::QUICK_FLAG);
        h.frame();
        assert!(!message(ids[2]).flagged);
    }

    /// Delete in Trash (a delete on the server) or Junk asks first, with
    /// no default button; elsewhere it moves to Trash at once.
    #[test]
    fn deleting_from_trash_or_junk_asks_first() {
        let h = Harness::new("delete-ask");
        more_messages(&h, &[("Trash", Role::Trash), ("Junk", Role::Junk)]);
        let ui = h.ui();
        let mut db = Db::open(&h.path).unwrap();
        let (account, trash, junk) = h.app(|a| {
            let by = |r| a.folders.iter().find(|f| f.role == r).unwrap().id;
            (a.accounts[0].id, by(Role::Trash), by(Role::Junk))
        });
        let header = |uid: u32| NewHeader {
            uid,
            subject: format!("old {uid}"),
            date: 1_700_000_000 + i64::from(uid),
            ..Default::default()
        };
        db.insert_headers(account, trash, &[header(20)]).unwrap();
        db.insert_headers(account, junk, &[header(21)]).unwrap();

        h.app(|a| a.set_view(View::Folder(trash)));
        let id = h.app(|a| a.rows.id_at(0)).unwrap();
        ui.invoke_message_selected(id as i32);
        h.frame();
        ui.invoke_delete();
        h.frame();
        assert_eq!(ui.get_q_title(), "Delete 1 message for good?");
        assert!(ui.get_q_destructive(), "no default button");
        assert!(h.app(|a| a.held.is_none()));
        // Cancelled: nothing moves.
        ui.invoke_q_answer(false);
        h.frame();
        assert!(h.app(|a| a.held.is_none() && a.rows.index_of(id).is_some()));
        // Delete (the key) and confirmed: a held delete on the server.
        ui.invoke_focus_list();
        h.keys(&[], &key(Key::Delete));
        assert_ne!(ui.get_q_title(), "");
        ui.invoke_q_answer(true);
        h.frame();
        h.app(|a| {
            assert!(matches!(
                a.held.as_ref().unwrap().jobs[..],
                [(_, Job::Delete { folder_id, .. })] if folder_id == trash
            ));
        });
        h.app(|a| a.flush_held());

        // Junk: asks, then moves to Trash.
        h.app(|a| a.set_view(View::Folder(junk)));
        let id = h.app(|a| a.rows.id_at(0)).unwrap();
        ui.invoke_message_selected(id as i32);
        h.frame();
        ui.invoke_delete();
        h.frame();
        assert_eq!(ui.get_q_title(), "Delete 1 message from Junk?");
        ui.invoke_q_answer(true);
        h.frame();
        h.app(|a| {
            assert!(matches!(
                a.held.as_ref().unwrap().jobs[..],
                [(_, Job::Move { dest, .. })] if dest == trash
            ));
        });
    }

    /// Backspace archives only from a message row or the reader's page:
    /// never in Drafts or the Outbox, nor with the focus on the filters.
    #[test]
    fn backspace_does_nothing_in_drafts_outbox_or_the_filters() {
        let h = Harness::new("backspace-guard");
        let ids = more_messages(&h, &[("Archive", Role::Archive)]);
        let ui = h.ui();
        let open = || h.app(|a| a.open.as_ref().map(|o| o.msg.id));
        for view in [View::Drafts, View::Outbox] {
            h.app(|a| a.set_view(view));
            ui.invoke_focus_list();
            h.keys(&[], &key(Key::DownArrow));
            h.keys(&[], &key(Key::Backspace));
            assert!(h.app(|a| a.held.is_none()), "{}", view.key());
        }
        h.app(|a| a.set_view(View::Folder(a.folders[0].id)));
        ui.invoke_message_selected(ids[0] as i32);
        h.frame();
        // Shift+Tab from the list reaches the filters (All, Unread,
        // Flagged): Backspace there changes nothing.
        ui.invoke_focus_list();
        h.keys(&[Key::Shift], &key(Key::Tab));
        h.keys(&[], &key(Key::Backspace));
        assert!(h.app(|a| a.held.is_none()));
        assert_eq!(open(), Some(ids[0]));
        // The arrows there move the filter, not the list.
        h.keys(&[], &key(Key::RightArrow));
        assert_eq!(ui.get_list_filter(), 1);
        h.app(|a| a.set_filter(0));
        // Back on the row: it archives.
        ui.invoke_focus_list();
        h.keys(&[], &key(Key::Backspace));
        assert_eq!(held_ids(&h), [ids[0]]);
    }

    /// The quick actions are no Tab stops: Tab from the ringed row goes to
    /// the reader's bar (Reply), not into the row.
    #[test]
    fn tab_from_a_row_skips_its_quick_actions() {
        let h = Harness::new("tab-quick");
        more_messages(&h, &[]);
        let ui = h.ui();
        ui.invoke_focus_list();
        // End opens the last row (the one with a body, so Reply opens at
        // once) with the keyboard ring (and its quick actions) on it.
        h.keys(&[], &key(Key::End));
        assert_eq!(
            h.app(|a| a.open.as_ref().map(|o| o.msg.id)),
            Some(h.message)
        );
        h.keys(&[], &key(Key::Tab));
        h.keys(&[], " ");
        assert!(ui.get_composing(), "Tab went to Reply");
        assert_eq!(ui.get_c_title(), "Reply");
    }

    /// Ctrl+Shift+F forwards, Ctrl+Shift+V opens Move (where it shows),
    /// Ctrl+Shift+I goes to the Inbox.
    #[test]
    fn forward_move_and_inbox_keys() {
        let h = Harness::new("shift-keys");
        more_messages(&h, &[("Archive", Role::Archive)]);
        let ui = h.ui();
        // The message with a body (Forward opens at once).
        ui.invoke_message_selected(h.message as i32);
        h.frame();
        ui.invoke_focus_list();
        h.keys(&[Key::Control, Key::Shift], "f");
        assert!(ui.get_composing());
        assert_eq!(ui.get_c_title(), "Forward");
        ui.invoke_c_close();
        h.frame();
        // Ctrl+Shift+V: the Move list opens (its popup reports itself).
        ui.invoke_focus_list();
        assert!(!ui.get_popup_open());
        h.keys(&[Key::Control, Key::Shift], "v");
        assert!(ui.get_popup_open(), "the Move flyout is open");
        // Ctrl+Shift+I from another view: the Inbox (one account).
        h.app(|a| a.set_view(View::Drafts));
        ui.invoke_focus_list();
        h.keys(&[Key::Control, Key::Shift], "i");
        assert!(h.app(|a| a.view == View::Folder(a.folders[0].id)));
    }

    /// The Move flyout: Drafts folders left out, depth kept, the last
    /// used folders first, and a filter.
    #[test]
    fn the_move_flyout_lists_recent_folders_and_filters() {
        let h = Harness::new("move-flyout");
        let ids = more_messages(
            &h,
            &[
                ("Archive", Role::Archive),
                ("Drafts", Role::Drafts),
                ("Projects", Role::Other),
                ("Receipts", Role::Other),
            ],
        );
        let db = Db::open(&h.path).unwrap();
        let account = h.app(|a| a.accounts[0].id);
        db.upsert_folder(account, "Projects/Komorebi", "Komorebi", 1, Role::Other)
            .unwrap();
        db.upsert_folder(account, "INBOX/Invoices", "Invoices", 1, Role::Other)
            .unwrap();
        h.app(|a| a.load_accounts().unwrap());
        let ui = h.ui();
        let targets = || -> Vec<(String, i32, bool)> {
            ui.get_move_targets()
                .iter()
                .map(|f| (f.label.to_string(), f.level, f.recent))
                .collect()
        };
        ui.invoke_message_selected(ids[0] as i32);
        h.frame();
        let all = targets();
        assert!(
            all.iter().all(|t| t.0 != "Drafts" && t.0 != "Inbox"),
            "{all:?}"
        );
        assert!(all.contains(&("Komorebi".into(), 1, false)), "{all:?}");
        // A child of the current folder (left out) is drawn flush.
        assert!(all.contains(&("Invoices".into(), 0, false)), "{all:?}");
        assert!(all.iter().all(|t| !t.2), "nothing used yet");
        // Move one to Receipts: it is the first recent folder after.
        let receipts = h.app(|a| {
            a.folders
                .iter()
                .find(|f| f.display == "Receipts")
                .unwrap()
                .id
        });
        ui.invoke_move_to(receipts as i32);
        h.frame();
        assert_eq!(h.undo(), "Moved to Receipts");
        let first = targets()[0].clone();
        assert_eq!(first, ("Receipts".into(), 0, true));
        assert_eq!(
            h.app(|a| a.db.settings().unwrap().move_recent),
            receipts.to_string()
        );
        // The filter narrows the list (no recent section then).
        ui.invoke_move_filter_edited("PRO".into());
        h.frame();
        assert_eq!(targets(), [("Projects".to_string(), 0, false)]);
        ui.invoke_move_filter_edited("zz".into());
        h.frame();
        assert!(targets().is_empty());
        assert!(ui.get_move_possible(), "a filter never turns Move off");
        // Menus open without the filter.
        h.app(|a| a.menu_opening());
        assert!(!targets().is_empty());
    }

    /// "Select messages" turns check boxes on (checked in the menu); a
    /// click then checks a row without opening it; Clear selection and
    /// Esc step back.
    #[test]
    fn select_messages_checks_rows_without_opening_them() {
        use slint::platform::PointerEventButton;
        let h = Harness::new("select-mode");
        let ids = more_messages(&h, &[]);
        let ui = h.ui();
        let select = h
            .app(|a| a.list_menu())
            .into_iter()
            .find(|e| e.title == "Select messages")
            .unwrap();
        assert!(select.checkable && !select.checked && select.enabled);
        ui.invoke_list_command(select.command);
        h.frame();
        assert!(ui.get_select_mode());
        assert!(h.app(|a| a.list_menu())[1].checked);
        let click = |row: f32| {
            let position = slint::LogicalPosition::new(400.0, 140.0 + 76.0 * row);
            let w = ui.window();
            w.dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
            w.dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
            h.frame();
        };
        click(2.5);
        click(3.5);
        assert_eq!(ui.get_selected_count(), 2);
        assert!(!ui.get_has_message(), "checked, not opened");
        assert!(!h.app(|a| a.db.message(ids[2]).unwrap().unwrap().seen));
        // The panel's Select all and Clear selection.
        ui.invoke_select_all_rows();
        h.frame();
        assert_eq!(ui.get_selected_count(), 6);
        ui.invoke_clear_selection();
        h.frame();
        assert_eq!(ui.get_selected_count(), 0);
        assert!(ui.get_select_mode(), "Select stays on");
        // Esc with nothing selected turns it off; so does another view.
        ui.invoke_focus_list();
        h.keys(&[], &key(Key::Escape));
        assert!(!ui.get_select_mode());
        ui.invoke_list_command(select.command);
        h.app(|a| a.set_view(View::Drafts));
        assert!(!ui.get_select_mode());
    }

    /// A message opened from the Unread filter stays listed (read by now)
    /// until the filter or the view changes, as in Outlook.
    #[test]
    fn a_message_opened_in_unread_stays_until_the_view_changes() {
        let h = Harness::new("kept-unread");
        let ids = more_messages(&h, &[]);
        let ui = h.ui();
        ui.invoke_set_filter(1);
        h.frame();
        let n = h.app(|a| a.rows.len());
        ui.invoke_message_selected(ids[0] as i32);
        h.frame();
        assert!(h.app(|a| a.db.message(ids[0]).unwrap().unwrap().seen));
        // A reload (a sync) keeps it.
        h.app(|a| a.reload_rows());
        assert!(h.app(|a| a.rows.index_of(ids[0]).is_some()));
        assert_eq!(h.app(|a| a.rows.len()), n);
        // Another filter (and back) drops it.
        ui.invoke_set_filter(0);
        ui.invoke_set_filter(1);
        h.frame();
        assert!(h.app(|a| a.rows.index_of(ids[0]).is_none()));
        assert_eq!(h.app(|a| a.rows.len()), n - 1);
    }

    /// The Undo offer has its own slot: a move's window is 10 s, stops
    /// while hovered, and the next move sends the held one.
    #[test]
    fn the_undo_offer_holds_its_window_and_the_next_move_flushes_it() {
        let h = Harness::new("undo-slot");
        let ids = more_messages(&h, &[("Archive", Role::Archive)]);
        let ui = h.ui();
        let cached = |id| h.app(|a| a.db.message(id).unwrap().is_some());
        ui.invoke_message_selected(ids[0] as i32);
        h.frame();
        ui.invoke_archive();
        h.frame();
        assert_eq!(h.undo(), "Moved to Archive");
        h.app(|a| {
            assert!(a.undo_timer.running());
            assert_eq!(a.undo_timer.interval(), triage::UNDO_WINDOW);
        });
        // A status line (a sync problem) doesn't take its place.
        h.app(|a| a.set_status("Something else", true));
        assert_eq!(h.undo(), "Moved to Archive");
        // Hovered: the window stops; left: it goes on (3 s at least).
        ui.invoke_undo_hold(true);
        h.frame();
        h.app(|a| assert!(!a.undo_timer.running()));
        ui.invoke_undo_hold(false);
        h.frame();
        h.app(|a| assert!(a.undo_timer.running() && a.held.is_some()));
        // The next archive sends the first and offers its own Undo.
        ui.invoke_archive();
        h.frame();
        assert!(!cached(ids[0]), "the first archive went to the server");
        assert_eq!(held_ids(&h), [ids[1]]);
        // Ctrl+Z takes the last one back.
        ui.invoke_focus_list();
        h.keys(&[Key::Control], "z");
        assert!(held_ids(&h).is_empty());
        assert_eq!(h.undo(), "");
    }

    /// The row and More menus carry their accelerator text, and an offline
    /// message not downloaded says so.
    #[test]
    fn menus_carry_accelerators_and_a_missing_body_says_why() {
        let h = Harness::new("menus");
        more_messages(&h, &[("Archive", Role::Archive)]);
        let ui = h.ui();
        h.app(|a| a.select_message(h.message));
        let (row, more) = h.app(|a| {
            a.menu_opening();
            (a.row_menu(), a.more_menu())
        });
        let titles = |groups: &[Vec<MenuCommand>]| -> Vec<String> {
            groups
                .iter()
                .flatten()
                .map(|e| e.title.to_string())
                .collect()
        };
        let row = titles(&row.groups);
        for want in [
            "Reply\tCtrl+R",
            "Reply all\tCtrl+Shift+R",
            "Forward\tCtrl+Shift+F",
            "Mark as unread\tCtrl+U",
            "Mark as important\tCtrl+Shift+G",
            "Archive\tBackspace",
            "Delete\tDel",
            "Save as .eml\tCtrl+S",
            "Show original",
        ] {
            assert!(row.iter().any(|t| t == want), "{want:?} in {row:?}");
        }
        let more = titles(&more.groups);
        assert!(more.iter().any(|t| t == "View as plain text"), "{more:?}");
        assert!(more.iter().any(|t| t == "Load images"), "{more:?}");
        assert!(!more.iter().any(|t| t.starts_with("Reply")), "{more:?}");
        // No worker in the harness: a message without its body is out of
        // reach, and the reader says so.
        h.app(|a| {
            a.open.as_mut().unwrap().msg.raw = None;
            a.show_open();
        });
        assert!(ui.get_loading() && ui.get_r_unreachable());
    }

    #[test]
    fn escape_dismisses_the_status_line_last() {
        let h = Harness::new("toast-esc");
        let ui = h.ui();
        ui.invoke_focus_list();
        h.app(|a| a.set_status("Moved to Archive", false));
        h.keys(&[], &key(Key::Escape));
        assert_eq!(h.status(), "");
        // The Undo offer stays: Esc never takes it away.
        h.app(|a| a.offer_undo("Moved to Archive", UndoOffer::Move(0)));
        h.keys(&[], &key(Key::Escape));
        assert_eq!(h.undo(), "Moved to Archive");
        h.app(|a| a.clear_undo(UndoOffer::Move(0)));
        // An open message closes first.
        let id = h.message;
        h.app(|a| {
            a.set_status("Saved", false);
            a.select_message(id);
        });
        h.keys(&[], &key(Key::Escape));
        assert!(!ui.get_has_message());
        assert_eq!(h.status(), "Saved");
        // Shift+F10 opens the row menu for what is selected (its entries
        // are built first).
        h.app(|a| a.select_message(id));
        ui.set_row_menu_2(ModelRc::default());
        h.keys(&[Key::Shift], &key(Key::F10));
        assert!(menu_has(ui, "Mark as unread"));
    }

    #[test]
    fn shift_f6_reaches_the_folders_from_every_pane() {
        let h = Harness::new("f6-folders");
        let ui = h.ui();
        let id = h.message;
        let items = ui.get_nav_items();
        let folder = (0..items.row_count())
            .find(|&i| items.row_data(i).unwrap().kind == 2)
            .expect("a folder row");
        let inbox = items.row_data(folder).unwrap().id;
        // From the list, the command bar and the page (Enter), with the
        // message open at 1280 px (the bar shows its labels): the folders
        // take the arrows and Enter.
        // Each key with Shift held or not.
        let (tab, f6, back, enter) = (
            (false, Key::Tab),
            (false, Key::F6),
            (true, Key::F6),
            (false, Key::Return),
        );
        let starts = [
            ("list", vec![back]),
            ("bar", vec![tab, back, back]),
            ("bar F6", vec![tab, f6, f6]),
            ("page", vec![enter, back, back]),
            ("page F6", vec![enter, f6, f6]),
        ];
        for (name, presses) in starts {
            h.app(|a| a.select_message(id));
            ui.invoke_focus_list();
            for (shift, k) in presses {
                h.keys(if shift { &[Key::Shift] } else { &[] }, &key(k));
            }
            h.keys(&[], &key(Key::Home));
            for _ in 0..folder {
                h.keys(&[], &key(Key::DownArrow));
            }
            h.keys(&[], &key(Key::Return));
            assert_eq!(ui.get_selected_folder(), inbox, "from the {name}");
            // Away from the Inbox again, to a view that lists the message.
            h.app(|a| a.set_view(View::AllInboxes));
        }
    }

    #[test]
    fn show_original_gives_the_focus_back_to_its_opener() {
        let h = Harness::new("src-return");
        let ui = h.ui();
        let id = h.message;
        h.app(|a| a.select_message(id));
        ui.invoke_focus_list();
        // Show original from More with the command bar's Reply focused,
        // then Esc: Reply has the keyboard again.
        h.keys(&[], &key(Key::Tab));
        ui.invoke_more_command(triage::CMD_ORIGINAL);
        h.frame();
        assert!(ui.get_src_show());
        h.keys(&[], &key(Key::Escape));
        assert!(!ui.get_src_show());
        h.keys(&[], " ");
        assert_eq!(ui.get_c_title(), "Reply");
    }

    // --------------------------------- teaching tips and What's new (Stage 5)

    use crate::app::tips;
    use komorebi::onboard::Tip;

    fn tip(h: &Harness) -> (i32, i32, i32) {
        let ui = h.ui();
        (ui.get_tip_kind(), ui.get_tip_n(), ui.get_tip_of())
    }

    /// Below 720 px one pane shows at a time: the list, or an open message
    /// with its back button (the list comes back with Esc or Back); a
    /// selection alone keeps the list. Below 1040 px the list stretches
    /// while the reader has nothing to show.
    #[test]
    fn narrow_windows_show_one_pane_at_a_time() {
        let h = Harness::new("one-pane");
        let ui = h.ui();
        let panes = |ui: &AppWindow| (ui.get_show_list(), ui.get_show_reader());
        assert_eq!(panes(ui), (true, true), "desktop: both");
        // Compact: the list alone while nothing is open.
        ui.window().set_size(slint::PhysicalSize::new(900, 700));
        h.frame();
        assert!(ui.get_compact() && !ui.get_one_pane());
        assert_eq!(panes(ui), (true, false));
        h.app(|a| a.select_message(h.message));
        h.frame();
        assert_eq!(panes(ui), (true, true), "compact with a message");
        // One pane: the message alone, then back to the list.
        ui.window().set_size(slint::PhysicalSize::new(560, 700));
        h.frame();
        assert!(ui.get_one_pane());
        assert_eq!(panes(ui), (false, true));
        ui.invoke_back_to_list();
        h.frame();
        assert!(!ui.get_has_message());
        assert_eq!(panes(ui), (true, false));
        // Esc is the same step.
        h.app(|a| a.select_message(h.message));
        h.frame();
        assert_eq!(panes(ui), (false, true));
        // The back button took the focus from the hidden list, so the
        // keys still work.
        h.keys(&[], "\u{1b}");
        h.frame();
        assert!(!ui.get_has_message());
        assert_eq!(panes(ui), (true, false));
        // A selection without an open message keeps the list.
        ui.invoke_message_toggled(h.message as i32);
        h.frame();
        assert!(!ui.get_has_message() && ui.get_selected_count() == 1);
        assert_eq!(panes(ui), (true, false));
        // The arrows only move the cursor while the list shows (opening
        // would hide it under the keys); Enter opens the row, and with a
        // message open the arrows step to the next one. Rows are listed
        // newest first, so Home is the newest.
        h.app(|a| a.clear_selection());
        let (account, inbox) = h.app(|a| (a.accounts[0].id, a.folders[0].id));
        let older: Vec<NewHeader> = (2..=3)
            .map(|uid| NewHeader {
                uid,
                subject: format!("Older {uid}"),
                date: 1_700_000_000 - i64::from(uid),
                ..Default::default()
            })
            .collect();
        Db::open(&h.path)
            .unwrap()
            .insert_headers(account, inbox, &older)
            .unwrap();
        h.app(|a| a.reload_rows());
        assert_eq!(h.app(|a| a.rows.len()), 3);
        ui.invoke_focus_list();
        h.keys(&[], &key(Key::Home));
        assert!(!ui.get_has_message(), "Home in one pane only selects");
        assert_eq!(ui.get_selected_count(), 1);
        assert_eq!(panes(ui), (true, false));
        h.keys(&[], &key(Key::DownArrow));
        assert!(!ui.get_has_message(), "Down in one pane only selects");
        let second = h.app(|a| a.rows.selection().cursor());
        assert_eq!(second, h.app(|a| a.rows.id_at(1)));
        h.keys(&[], &key(Key::Return));
        assert!(ui.get_has_message());
        assert_eq!(h.app(|a| a.open.as_ref().map(|o| o.msg.id)), second);
        assert_eq!(panes(ui), (false, true));
        h.keys(&[], &key(Key::DownArrow));
        assert_eq!(
            h.app(|a| a.open.as_ref().map(|o| o.msg.id)),
            h.app(|a| a.rows.id_at(2)),
            "Down with a message open steps to the next"
        );
        h.keys(&[], "\u{1b}");
        assert!(!ui.get_has_message());
        assert_eq!(panes(ui), (true, false));
        // The composer takes the pane; closing it brings the list back.
        h.app(|a| a.clear_selection());
        h.keys(&[Key::Control], "n");
        h.frame();
        assert!(ui.get_composing());
        assert_eq!(panes(ui), (false, true));
        // An Outbox entry takes the pane too; Back closes it.
        h.keys(&[], "\u{1b}");
        assert!(!ui.get_composing());
        let failed = settled_row(&h, db::Outbox::Failed, "transient error (421): later");
        ui.invoke_nav_clicked(NavItem {
            kind: 4,
            ..Default::default()
        });
        h.frame();
        assert_eq!(panes(ui), (true, false));
        // The arrows only pick an Outbox row too; Enter opens it.
        ui.invoke_focus_list();
        h.keys(&[], &key(Key::DownArrow));
        assert!(!ui.get_o_show(), "Down in the one-pane Outbox only selects");
        assert_eq!(panes(ui), (true, false));
        h.keys(&[], &key(Key::Return));
        assert!(ui.get_o_show());
        assert_eq!(panes(ui), (false, true));
        ui.invoke_back_to_list();
        h.frame();
        assert!(!ui.get_o_show());
        ui.invoke_message_selected(failed as i32);
        h.frame();
        assert!(ui.get_o_show());
        assert_eq!(panes(ui), (false, true));
        ui.invoke_back_to_list();
        h.frame();
        assert!(!ui.get_o_show());
        assert_eq!(panes(ui), (true, false));
        ui.window().set_size(slint::PhysicalSize::new(1280, 800));
        h.frame();
        assert!(!ui.get_one_pane() && !ui.get_compact());
        assert_eq!(panes(ui), (true, true));
    }

    fn seen(h: &Harness) -> String {
        h.app(|a| a.db.settings().unwrap().tips_seen)
    }

    fn started(h: &Harness) -> Option<(Tip, usize, usize)> {
        h.app(|a| a.start_tips()).map(|s| (s.tip, s.n, s.of))
    }

    #[test]
    fn tips_follow_the_layout_and_esc_closes_only_the_tip() {
        let h = Harness::new("tips");
        let ui = h.ui();
        assert!(!ui.get_compact());
        // The desktop: Search, then Help (2 tips).
        assert_eq!(started(&h), Some((Tip::Search, 1, 2)));
        assert_eq!(tip(&h), (2, 1, 2));
        assert!(ui.get_tip_visible());

        // A search typed and a message open: Esc closes the tip only (Got
        // it), the innermost step of the Esc stack.
        ui.set_search_text("Lunch".into());
        ui.invoke_search_edited("Lunch".into());
        h.app(|a| a.reload_rows());
        h.app(|a| a.select_message(h.message));
        assert!(ui.get_has_message());
        h.keys(&[], "\u{1b}");
        assert_eq!(tip(&h), (3, 2, 2));
        assert_eq!(ui.get_search_text(), "Lunch");
        assert!(ui.get_has_message());
        assert_eq!(seen(&h), "search");
        h.keys(&[], "\u{1b}");
        assert_eq!(tip(&h).0, 0);
        assert!(!ui.get_tip_visible());
        assert_eq!(ui.get_search_text(), "Lunch");
        assert!(ui.get_has_message());
        assert_eq!(seen(&h), "search,help");
        // With no tip, Esc is the next step: the search clears.
        h.keys(&[], "\u{1b}");
        assert_eq!(ui.get_search_text(), "");
        // Nothing more on the desktop.
        assert_eq!(started(&h), None);

        // The window folds into the rail: Folders alone, 1 of 1.
        ui.window().set_size(slint::PhysicalSize::new(900, 700));
        h.frame();
        assert!(ui.get_compact());
        assert_eq!(tip(&h).0, 0, "nothing starts by itself in tests");
        assert_eq!(started(&h), Some((Tip::Folders, 1, 1)));
        assert!(ui.get_tip_visible());
        // The pane opened over the content hides it (its target is in use).
        ui.set_pane_open(true);
        h.frame();
        assert!(!ui.get_tip_visible());
        ui.set_pane_open(false);
        h.frame();
        assert!(ui.get_tip_visible());
        // So does an in-window popup (the Move list): Esc then closes the
        // popup, not the tip.
        ui.set_popup_open(true);
        h.frame();
        assert!(!ui.get_tip_visible());
        ui.set_popup_open(false);
        h.frame();
        assert!(ui.get_tip_visible());
        // The Keyboard shortcuts dialog hides it too, and keeps the keys:
        // Ctrl+N behind its scrim opens nothing; Esc closes the dialog
        // only, and the tip is back.
        ui.invoke_open_shortcuts();
        h.frame();
        assert!(ui.get_shortcuts_show());
        assert!(!ui.get_tip_visible());
        h.keys(&[Key::Control], "n");
        assert!(!ui.get_composing());
        assert!(ui.get_shortcuts_show());
        h.keys(&[], "\u{1b}");
        h.frame();
        assert!(!ui.get_shortcuts_show());
        assert!(ui.get_tip_visible());
        assert_eq!(tip(&h), (1, 1, 1));
        // Wide again: the Folders tip goes with the rail, still unseen.
        ui.window().set_size(slint::PhysicalSize::new(1280, 800));
        h.frame();
        assert!(!ui.get_compact());
        assert_eq!(tip(&h).0, 0);
        assert_eq!(seen(&h), "search,help");
    }

    #[test]
    fn the_compact_series_has_three_tips_and_skip_ends_both_layouts() {
        let h = Harness::new("tips-compact");
        let ui = h.ui();
        ui.window().set_size(slint::PhysicalSize::new(900, 700));
        h.frame();
        assert_eq!(started(&h), Some((Tip::Folders, 1, 3)));
        h.app(|a| a.tip_got_it());
        assert_eq!(tip(&h), (2, 2, 3));
        assert_eq!(seen(&h), "folders");
        // Skip tips: every tip seen, in either layout.
        h.app(|a| a.tip_skip());
        assert_eq!(tip(&h).0, 0);
        assert_eq!(seen(&h), "folders,search,help");
        assert_eq!(started(&h), None);
        ui.window().set_size(slint::PhysicalSize::new(1280, 800));
        h.frame();
        assert_eq!(started(&h), None);
        // Settings > About > Show tips again: due again, Settings closed.
        ui.set_show_settings(true);
        h.frame();
        ui.invoke_show_tips_again();
        h.frame();
        assert!(!ui.get_show_settings());
        assert_eq!(seen(&h), "");
        assert_eq!(started(&h), Some((Tip::Search, 1, 2)));
        // Never over Settings.
        ui.set_show_settings(true);
        h.frame();
        assert!(!ui.get_tip_visible());
        ui.set_show_settings(false);
        h.frame();
        assert!(ui.get_tip_visible());
    }

    #[test]
    fn whats_new_shows_once_and_never_with_a_tip() {
        let h = Harness::new("whats-new");
        let ui = h.ui();
        // An update from before the bar: an account, no version saved.
        h.app(|a| a.onboarding = tips::onboarding(&a.db.settings().unwrap(), true));
        // A tip up: the bar waits for the series to end.
        assert!(started(&h).is_some());
        assert!(!h.app(|a| a.show_whats_new_if_due()));
        assert_eq!(h.status(), "");
        h.app(|a| a.tip_skip());
        assert!(h.app(|a| a.show_whats_new_if_due()));
        assert_eq!(
            h.status(),
            format!("Komorebi was updated to {}.", tips::VERSION)
        );
        assert_eq!(ui.get_status_action(), "What's new");
        let saved = || h.app(|a| a.db.settings().unwrap().whats_new_version);
        // Not told yet: saved only once closed or acted on.
        assert_eq!(saved(), None);
        // While it is up, no tip starts.
        h.app(|a| a.onboarding.reset());
        assert_eq!(started(&h), None);
        // Its action opens the changelog (through the link prompt), and
        // this version is told.
        ui.invoke_status_action_clicked();
        h.frame();
        assert_eq!(h.status(), "");
        assert!(ui.get_l_show());
        assert_eq!(saved().as_deref(), Some(tips::VERSION));
        ui.invoke_l_cancel();
        h.frame();
        // Gone, the tips may start; the bar never shows again.
        assert!(started(&h).is_some());
        h.app(|a| a.tip_skip());
        assert!(!h.app(|a| a.show_whats_new_if_due()));
        // The next start, with this version saved: not due.
        let settings = h.app(|a| a.db.settings().unwrap());
        assert!(!tips::onboarding(&settings, true).bar_ready());
    }

    #[test]
    fn whats_new_replaced_by_another_status_comes_back_next_start() {
        let h = Harness::new("whats-new-replaced");
        h.app(|a| a.onboarding = tips::onboarding(&a.db.settings().unwrap(), true));
        assert!(h.app(|a| a.show_whats_new_if_due()));
        // A server out of reach (or any status) takes the slot.
        h.app(|a| a.set_status("Can't reach the server.", true));
        assert_eq!(h.status(), "Can't reach the server.");
        // Not again this run, but the next start shows it.
        assert!(!h.app(|a| a.show_whats_new_if_due()));
        let settings = h.app(|a| a.db.settings().unwrap());
        assert_eq!(settings.whats_new_version, None);
        assert!(tips::onboarding(&settings, true).bar_ready());
        // Closed: told.
        h.app(|a| {
            a.onboarding = tips::onboarding(&a.db.settings().unwrap(), true);
            a.show_whats_new_if_due();
            a.dismiss_status();
        });
        let settings = h.app(|a| a.db.settings().unwrap());
        assert_eq!(settings.whats_new_version.as_deref(), Some(tips::VERSION));
    }

    #[test]
    fn the_first_account_saves_the_version_silently() {
        let h = Harness::new("whats-new-first");
        fake_store(&h);
        // No account yet (the harness's removed), no version saved.
        h.app(|a| {
            let id = a.accounts[0].id;
            a.db.remove_account(id).unwrap();
            a.load_accounts().unwrap();
            a.onboarding = tips::onboarding(&a.db.settings().unwrap(), false);
        });
        assert!(h.app(|a| a.accounts.is_empty()));
        assert_eq!(h.app(|a| a.db.settings().unwrap().whats_new_version), None);
        h.app(|a| {
            a.finish_add_account(
                new_account("ana@example.net"),
                String::new(),
                "pw".into(),
                (None, None),
            )
        });
        let saved = h.app(|a| a.db.settings().unwrap().whats_new_version);
        assert_eq!(saved.as_deref(), Some(tips::VERSION));
        assert!(!h.app(|a| a.show_whats_new_if_due()));
        // The flow says the rest (no status line).
        assert_eq!(h.status(), "");
        assert_eq!(h.ui().get_ob_step(), 3);
    }
}
