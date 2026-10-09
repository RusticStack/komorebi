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

mod markup;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct DraftId(u64);

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

    fn title(self) -> &'static str {
        match self {
            Kind::New => "New message",
            Kind::Reply => "Reply",
            Kind::ReplyAll => "Reply all",
            Kind::Forward => "Forward",
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
    /// Inline errors for To, Cc and Bcc (empty: none shown).
    pub errors: [String; 3],
    /// The signature block the app put in the body (`signature_block`;
    /// empty: none). `None` for a saved draft whose body lacks its
    /// account's current block: never swapped.
    pub signature: Option<String>,
    /// A reply's or forward's quote of the original's HTML, for the HTML
    /// part (`compose_html::attach`); the body holds the text quote.
    pub quoted_html: Option<String>,
    /// The last formatting edit, for Ctrl+Z (not saved).
    pub unformat: Option<Unformat>,
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
    pub fn reply(m: &db::Message, body: &Body, me: &str, all: bool) -> Self {
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
                attribution(m),
                quote(strip_signature(&body.text))
            ),
            reply_to: Some(m.id),
            in_reply_to: m.msgid.clone(),
            references: body.references.clone(),
            ..Default::default()
        }
    }

    /// A forward of `m` with its listed attachments.
    pub fn forward(m: &db::Message, body: &Body, attachments: Vec<Attached>) -> Self {
        Draft {
            account_id: m.account_id,
            kind: Kind::Forward,
            subject: prefixed("Fwd:", &m.subject),
            body: format!(
                "\n\n{FORWARDED}\nFrom: {} <{}>\nDate: {}\nSubject: {}\nTo: {}\n\n{}",
                m.from_name,
                m.from_addr,
                format::full_date(m.date),
                m.subject,
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
    pub fn quote_html(&mut self, m: &db::Message, original: &str) {
        use mail::compose_html;
        let (boundary, block) = match self.kind {
            Kind::New => return,
            Kind::Reply | Kind::ReplyAll => {
                let line = attribution(m);
                let block = compose_html::reply_quote(&line, original);
                (line, block)
            }
            Kind::Forward => {
                let from = format!("{} <{}>", m.from_name, m.from_addr);
                let date = format::full_date(m.date);
                let fields = [
                    ("From", from.trim()),
                    ("Sent", date.as_str()),
                    ("To", m.to.as_str()),
                    ("Subject", m.subject.as_str()),
                ];
                let block = compose_html::forward_quote(FORWARDED, &fields, original);
                (FORWARDED.to_string(), block)
            }
        };
        self.quoted_html = compose_html::attach(&self.body, &boundary, &block);
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
            body: self.body.clone(),
            quoted_html: self.quoted_html.clone(),
            message_id: self.message_id.clone(),
            updated: 0,
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
        if let Some(i) = address_index(field)
            && !self.errors[i].is_empty()
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
        let sig = self.signature.as_deref().unwrap_or_default();
        let body = match self.body.find(sig) {
            Some(i) if !sig.is_empty() => {
                format!("{}{}", &self.body[..i], &self.body[i + sig.len()..])
            }
            _ => self.body.clone(),
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

    /// Worth keeping when another draft takes its place: the user changed
    /// it. An untouched reply or forward is only what the app filled in.
    pub fn keep(&self) -> bool {
        self.touched && !self.is_empty()
    }

    /// Why Send is off, shown under the toolbar (empty: it is on).
    pub fn send_block(&self) -> String {
        if let Some(i) = self.errors.iter().position(|e| !e.is_empty()) {
            let name = ["To", "Cc", "Bcc"][i];
            return format!("Fix the {name} address to send.");
        }
        if [&self.to, &self.cc, &self.bcc]
            .iter()
            .all(|s| s.trim().is_empty())
        {
            return "Add a recipient to send.".into();
        }
        if encoded_size(attachments_size(&self.attachments)) > MAX_ATTACHMENTS_BYTES {
            return "Attachments are over 25 MB; remove some to send.".into();
        }
        String::new()
    }
}

fn address_index(field: ComposeField) -> Option<usize> {
    ADDRESS_FIELDS.iter().position(|f| *f == field)
}

/// The inline error for an address field as typed (empty: fine).
pub(super) fn address_error(text: &str) -> String {
    match mail::smtp::check_addresses(text) {
        Ok(()) => String::new(),
        // The context alone ("“x” is not a valid address"), not the
        // parser's detail.
        Err(e) => e.to_string(),
    }
}

#[derive(Default)]
pub(super) struct Drafts {
    items: HashMap<DraftId, Draft>,
    active: Option<DraftId>,
    next: u64,
    /// The recipient suggestions showing for the active draft.
    suggest: Option<Suggest>,
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
    /// worth keeping and is dropped otherwise (save it first).
    pub fn switch_to(&mut self, id: DraftId) {
        if !self.items.contains_key(&id) {
            return;
        }
        if let Some(old) = self.active
            && old != id
            && self.items.get(&old).is_some_and(|d| !d.keep())
        {
            self.items.remove(&old);
        }
        self.active = Some(id);
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
        self.items.remove(&id)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
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
/// none is attached), as the bar's text. Empty: nothing to ask about.
pub(super) fn send_guard(d: &Draft) -> String {
    let no_subject = d.subject.trim().is_empty();
    let no_attachment = d.attachments.is_empty() && mentions_attachment(&d.body);
    match (no_subject, no_attachment) {
        (true, true) => "No subject, and the text mentions an attachment but nothing is attached.",
        (true, false) => "This message has no subject.",
        (false, true) => "The text mentions an attachment, but nothing is attached.",
        (false, false) => "",
    }
    .into()
}

/// Whether the user's own text mentions an attachment, in English or
/// Portuguese ("attached", "attachment", "anexo", "em anexo", "anexado",
/// ...). Quoted lines (`>`), a forwarded message and everything from the
/// signature line (`-- `) on are not the user's.
pub(super) fn mentions_attachment(body: &str) -> bool {
    body.lines()
        .take_while(|l| !l.starts_with(FORWARDED) && *l != "-- ")
        .filter(|l| !l.trim_start().starts_with('>'))
        .flat_map(|l| l.split(|c: char| !c.is_alphanumeric()))
        .any(|w| {
            let w = w.to_lowercase();
            w.starts_with("attach") || w.starts_with("anex")
        })
}

/// The line a forward's quoted message starts with.
const FORWARDED: &str = "---------- Forwarded message ----------";

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
        !ui.get_composing() && !ui.get_show_settings()
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

    /// New mail. With the composer hidden it brings back the draft in
    /// progress (or the newest one kept); with it showing, that draft is
    /// kept and a new one starts.
    pub(super) fn compose_new(&mut self) {
        let Some(account) = self.current_account() else {
            return;
        };
        let ui = self.ui();
        if !ui.get_composing() {
            let resume = self
                .drafts
                .active_id()
                .filter(|_| self.drafts.active().is_some_and(Draft::keep))
                .or_else(|| self.drafts.newest_kept());
            if let Some(id) = resume {
                self.drafts.switch_to(id);
                return self.show_draft();
            }
        }
        self.start_draft(Draft::new(account));
    }

    /// A `mailto:` link was clicked: a new message with its fields, from
    /// the open message's account. A draft in progress is kept.
    pub(super) fn compose_mailto(&mut self, m: Mailto) {
        let Some(account) = self.current_account() else {
            return self.set_status("Add an account to write messages.", false);
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
        if open.body.is_none() {
            return self.set_status("The message is still downloading.", false);
        }
        // A message replied to already: its draft comes back.
        let message = open.msg.id;
        if let Some(id) = self.drafts.find_reply(message) {
            return self.resume_draft(id);
        }
        match self.db.draft_for_message(message) {
            Ok(Some(row)) => return self.open_saved(row),
            Ok(None) => {}
            Err(e) => return self.set_status(&format!("{e:#}"), true),
        }
        let Some(open) = &self.open else { return };
        let Some(body) = &open.body else { return };
        let me = self.account_email(open.msg.account_id);
        let mut draft = Draft::reply(&open.msg, body, &me, all);
        if let Some(html) = quotable(open.msg.raw.as_deref()) {
            draft.quote_html(&open.msg, &html);
        }
        self.start_draft(draft);
    }

    pub(super) fn forward(&mut self) {
        if !self.reading() {
            return;
        }
        let Some(open) = &self.open else { return };
        let (Some(body), Some(raw)) = (&open.body, open.msg.raw.as_deref()) else {
            return self.set_status("The message is still downloading.", false);
        };
        let attachments = mime::attachments(raw, |_| true)
            .into_iter()
            .map(|(filename, data)| OutAttachment { filename, data }.into())
            .collect();
        let mut draft = Draft::forward(&open.msg, body, attachments);
        if let Some(html) = quotable(Some(raw)) {
            draft.quote_html(&open.msg, &html);
        }
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
    /// saved and kept. A new draft gets its account's signature.
    fn start_draft(&mut self, mut draft: Draft) {
        let block = self.signature_of(draft.account_id);
        if draft.row.is_none() {
            draft.sign(&block);
        } else if block.is_empty() || draft.body.contains(&block) {
            // Saved earlier: swappable while it holds the current block.
            draft.signature = Some(block);
        }
        let parked = self.park();
        self.drafts.open(draft);
        self.show_draft();
        if parked {
            self.set_status("Earlier draft saved in Drafts.", false);
        }
    }

    /// Show draft `id`, already in memory, saving the one it replaces.
    fn resume_draft(&mut self, id: DraftId) {
        if self.drafts.active_id() != Some(id) {
            let parked = self.park();
            self.drafts.switch_to(id);
            if parked {
                self.set_status("Earlier draft saved in Drafts.", false);
            }
        }
        self.show_draft();
    }

    /// Open saved draft `row` in the composer (from the Drafts view, Resume
    /// draft, or a reply to its message).
    pub(super) fn open_saved(&mut self, row: i64) {
        if let Some(id) = self.drafts.find_row(row) {
            return self.resume_draft(id);
        }
        match self.db.is_saved_draft(row) {
            Ok(true) => {}
            Ok(false) => {
                self.drafts_changed(None, true);
                return self.set_status("This draft is no longer saved.", true);
            }
            Err(e) => return self.set_status(&format!("Couldn't open the draft: {e:#}"), true),
        }
        match load(&self.db, row) {
            Ok(Some((draft, missing))) => {
                self.start_draft(draft);
                if !missing.is_empty() {
                    let names = missing.join(", ");
                    self.set_status(
                        &format!("Couldn't load {names}; left out of this draft."),
                        true,
                    );
                }
            }
            Ok(None) => {
                self.drafts_changed(None, true);
                self.set_status("This draft is no longer saved.", true);
            }
            Err(e) => self.set_status(&format!("Couldn't open the draft: {e:#}"), true),
        }
    }

    /// Resume draft (the status line's action at start): the last saved.
    pub(super) fn resume_last_saved(&mut self) {
        self.set_status("", false);
        match self.db.draft_ids("") {
            Ok(ids) => {
                if let Some(&row) = ids.first() {
                    self.open_saved(row);
                }
            }
            Err(e) => self.set_status(&format!("{e:#}"), true),
        }
    }

    /// After the first paint: count the saved drafts and the outbox for
    /// the nav, and offer the last draft once (unless another notice shows).
    pub(super) fn offer_drafts(&mut self) {
        self.refresh_draft_count();
        self.refresh_outbox_count();
        self.rebuild_nav();
        let n = self.draft_count;
        if n > 0 && self.ui().get_status().is_empty() {
            let s = if n == 1 { "" } else { "s" };
            self.set_status(&format!("{n} saved draft{s}"), false);
            self.set_status_action("Resume draft", StatusAction::ResumeDraft);
        }
    }

    pub(super) fn refresh_draft_count(&mut self) {
        match self.db.count_drafts() {
            Ok(n) => self.draft_count = n,
            Err(e) => self.set_status(&format!("{e:#}"), true),
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
        }
        if self.view == View::Drafts {
            self.reload_rows();
        }
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
        match store(&db, d, &email) {
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
                self.set_status(&format!("Couldn't save the draft: {e:#}"), true);
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
    fn schedule_autosave(&mut self) {
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
        self.focus_field(focus);
        ui.set_composing(true);
        self.sync_selection_index();
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

    /// Ctrl+Z right after a formatting edit: the body as it was.
    pub(super) fn unformat_body(&mut self) -> BodyEdit {
        let Some(d) = self.drafts.active_mut() else {
            return BodyEdit::default();
        };
        let Some(u) = d.unformat.take_if(|u| u.after == d.body) else {
            return BodyEdit::default();
        };
        self.draft_edited(ComposeField::Body, &u.before);
        self.ui().set_c_body(u.before.as_str().into());
        body_edit(u.anchor, u.cursor)
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
        let (errors, block) = match self.drafts.active() {
            Some(d) => (d.errors.clone(), d.send_block()),
            None => Default::default(),
        };
        let [to, cc, bcc] = errors;
        ui.set_c_to_error(to.into());
        ui.set_c_cc_error(cc.into());
        ui.set_c_bcc_error(bcc.into());
        ui.set_c_send_reason(block.into());
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
                size: format::size(a.file.data.len()).into(),
            })
            .collect();
        let encoded = self
            .drafts
            .active()
            .map_or(0, |d| encoded_size(attachments_size(&d.attachments)));
        let ui = self.ui();
        ui.set_c_attachments(ModelRc::new(VecModel::from(rows)));
        ui.set_c_attach_total(
            format!(
                "Attachments {} of 25 MB",
                format::size(usize::try_from(encoded).unwrap_or(usize::MAX))
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
    }

    /// Set a field of the active draft and show it (as if typed).
    #[cfg(any(test, debug_assertions))]
    pub(super) fn set_draft_field(&mut self, field: ComposeField, text: &str) {
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
    /// untouched, and the draft is saved at once.
    pub(super) fn draft_account(&mut self, index: i32) {
        let Some(account) = self.accounts.get(index.max(0) as usize) else {
            return;
        };
        let (id, block) = (account.id, signature_block(&account.signature));
        let Some(d) = self.drafts.active_mut() else {
            return;
        };
        if d.account_id == id {
            return;
        }
        d.account_id = id;
        d.touched = true;
        d.dirty = true;
        if d.resign(&block) {
            let body = SharedString::from(d.body.as_str());
            self.ui().set_c_body(body);
        }
        if let Some(draft) = self.drafts.active_id() {
            self.save_draft(draft);
        }
    }

    /// The signature block of an account's mail (empty: none).
    fn signature_of(&self, account: i64) -> String {
        self.accounts
            .iter()
            .find(|a| a.id == account)
            .map_or_else(String::new, |a| signature_block(&a.signature))
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
                self.set_status(&format!("{e:#}"), true);
            }
            self.show_compose_attachments();
            self.show_send_state();
            self.save_draft(id);
        }
    }

    /// Esc: save the draft and hide the composer. A draft worth keeping
    /// stays for New mail and in Drafts; an empty one goes.
    pub(super) fn close_composer(&mut self) {
        if let Some(id) = self.drafts.active_id() {
            let saved = self.save_draft(id);
            if !self.drafts.get(id).is_some_and(Draft::keep) {
                self.drafts.remove(id);
            } else if saved {
                self.save_to_server(id, true);
                self.set_status("Draft saved", false);
            }
        }
        self.hide_composer();
    }

    fn hide_composer(&self) {
        let ui = self.ui();
        ui.set_c_suggestions(ModelRc::default());
        ui.set_composing(false);
        ui.invoke_focus_list();
        self.sync_selection_index();
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
                Err(e) => self.set_status(&format!("{e:#}"), true),
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
        if !block.is_empty() {
            self.show_send_state();
            return self.set_status(&block, true);
        }
        let guard = if anyway { String::new() } else { send_guard(d) };
        let account_id = d.account_id;
        self.ui().set_c_guard(guard.as_str().into());
        if !guard.is_empty() {
            return;
        }
        let Some(account) = self.accounts.iter().find(|a| a.id == account_id).cloned() else {
            return self.set_status("Add an account to send messages.", true);
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
            Ok(false) => return self.set_status("This draft is no longer saved.", true),
            Err(e) => return self.set_status(&format!("Couldn't send: {e:#}"), true),
        }
        let Some(d) = self.drafts.remove(id) else {
            return;
        };
        let reply_to = d.reply_to;
        self.hide_composer();
        self.drafts_changed(reply_to, true);
        self.queue_send(row, account_id, outgoing(&account, d), self.undo_delay);
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
        self.pending_sends.insert(
            row,
            PendingSend {
                seq,
                account,
                cancel: cancel.clone(),
                built: false,
                due: delay.is_zero(),
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
            self.set_status(&format!("Sending in {} s", delay.as_secs()), false);
            self.set_status_action("Undo", StatusAction::Undo(row));
        }
        self.outbox_changed();
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
                    return self.set_status(&format!("{e:#}"), true);
                }
                self.set_status(&format!("Couldn't send: {e}. It is back in Drafts."), true);
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
        if self.status_action.get() == StatusAction::Undo(row) {
            self.set_status("", false);
        }
        match self.db.outbox_queue(row) {
            Ok(true) => self.dispatch(row, p.account),
            Ok(false) => {}
            Err(e) => self.set_status(&format!("{e:#}"), true),
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
                self.set_status("", false);
                self.outbox_changed();
                self.open_saved(row);
                let reply_to = self.drafts.active().and_then(|d| d.reply_to);
                self.drafts_changed(reply_to, true);
            }
            Ok(false) => {
                self.pending_sends.remove(&row);
                self.set_status("Too late to undo: the message is being sent.", true);
            }
            // Still cancelled: it is never queued from here.
            Err(e) => self.set_status(&format!("Couldn't undo: {e:#}"), true),
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
}

impl PendingSend {
    /// As Undo does: a build finishing now stores nothing.
    pub(super) fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

/// What draft `d` sends from `account`: plain text, or with an HTML
/// alternative when it has formatting or quotes an HTML message.
pub(super) fn outgoing(account: &Account, d: Draft) -> Outgoing {
    let (body, body_html) = mail::compose_html::alternative(&d.body, d.quoted_html.as_deref());
    Outgoing {
        from_name: account.name.clone(),
        from_email: account.email.clone(),
        to: d.to,
        cc: d.cc,
        bcc: d.bcc,
        subject: d.subject,
        body,
        body_html,
        attachments: d.attachments.into_iter().map(|a| a.file).collect(),
        in_reply_to: d.in_reply_to,
        references: d.references,
        message_id: d.message_id,
        calendar: None,
    }
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
        .set_title("Attach files")
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
        if !ui.get_composing() || ui.get_show_settings() || paths.is_empty() {
            return;
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
            self.set_status(&format!("Couldn't attach the files: {e}"), true);
        }
    }

    /// Add files read for draft `target`, within the hard limit (another
    /// read may have finished first), and save their copies once.
    pub(super) fn add_attachments(
        &mut self,
        target: DraftId,
        added: Vec<Attached>,
        mut errors: Vec<String>,
    ) {
        let Some(d) = self.drafts.get_mut(target) else {
            return;
        };
        let mut total = attachments_size(&d.attachments);
        let mut any = false;
        for a in added {
            let len = a.file.data.len() as u64;
            if encoded_size(total + len) > MAX_ATTACH_HARD {
                errors.push(format!("{}: {}", a.file.filename, too_big()));
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
            self.set_status(&errors.join("; "), true);
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

/// Read files to attach when `total` bytes are attached already: the
/// attachments, and an error line for each file that can't be.
fn read_files(paths: &[PathBuf], mut total: u64) -> (Vec<Attached>, Vec<String>) {
    let mut added = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "attachment".into());
        if path.is_dir() {
            errors.push(format!("{name}: folders can't be attached"));
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
            Err(e) => errors.push(format!("{name}: {e}")),
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

fn too_big() -> std::io::Error {
    std::io::Error::other("over Komorebi's 100 MB attachment limit")
}

/// Read a file to attach when `total` bytes are attached already. A file that
/// would take the message past the hard limit is refused before reading.
fn read_attachment(path: &Path, total: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let len = std::fs::metadata(path)?.len();
    if encoded_size(total.saturating_add(len)) > MAX_ATTACH_HARD {
        return Err(too_big());
    }
    // The file may grow after the size check; never read past the limit.
    let limit = (MAX_ATTACH_HARD * 3 / 4).saturating_sub(total);
    let mut data = Vec::with_capacity(len as usize);
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > limit {
        return Err(too_big());
    }
    Ok(data)
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

/// The line a reply's quote starts with.
fn attribution(m: &db::Message) -> String {
    format!(
        "On {}, {} wrote:",
        format::full_date(m.date),
        format::sender(&m.from_name, &m.from_addr)
    )
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
    use slint::Rgb8Pixel;
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{Key, Platform, WindowAdapter, WindowEvent};

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
            raw: None,
        }
    }

    #[test]
    fn drafts_per_kind() {
        let new = Draft::new(7);
        assert_eq!((new.account_id, new.kind), (7, Kind::New));
        assert!(new.is_empty() && !new.keep());
        assert_eq!(new.send_block(), "Add a recipient to send.");

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
        let reply = Draft::reply(&m, &body, "al@x.org", false);
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
        let all = Draft::reply(&m, &body, "al@x.org", true);
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
        let fwd = Draft::forward(&m, &body, vec![file.into()]);
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
        assert_eq!(d.errors, [String::new(), String::new(), String::new()]);
        assert_eq!(d.send_block(), "");

        d.edit(ComposeField::Cc, "ana@x.pt, j@");
        assert!(d.errors[1].is_empty());
        d.validate(ComposeField::Cc);
        assert_eq!(d.errors[1], "“j@” is not a valid address");
        assert_eq!(d.send_block(), "Fix the Cc address to send.");
        // Once shown, the error follows the typing and clears when right.
        d.edit(ComposeField::Cc, "ana@x.pt, j@x");
        assert_eq!(d.errors[1], "");
        assert_eq!(d.send_block(), "");

        assert!(!address_error("João Silva j@x.pt").is_empty());
        assert!(address_error("\"Silva, João\" <j@x.pt>").is_empty());
        // Subject and body are never checked.
        d.edit(ComposeField::Subject, "<<<");
        d.validate(ComposeField::Subject);
        assert_eq!(d.send_block(), "");
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
        assert_eq!(send_guard(&d), "This message has no subject.");
        d.edit(ComposeField::Body, "The file is attached.");
        assert_eq!(
            send_guard(&d),
            "No subject, and the text mentions an attachment but nothing is attached."
        );
        d.edit(ComposeField::Subject, "  Report ");
        assert_eq!(
            send_guard(&d),
            "The text mentions an attachment, but nothing is attached."
        );
        d.attachments.push(Attached::from(OutAttachment {
            filename: "report.pdf".into(),
            data: vec![1],
        }));
        assert_eq!(send_guard(&d), "");
        // A forward with nothing attached is the user's choice to make.
        d.attachments.clear();
        d.edit(ComposeField::Body, "FYI");
        assert_eq!(send_guard(&d), "");
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
        let mut reply = Draft::reply(&m, &body, "ana@x.org", false);
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

        let mut fwd = Draft::forward(&m, &body, Vec::new());
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
        let mut r = Draft::reply(&m, &Body::default(), "ana@x.org", false);
        r.sign("");
        assert!(r.resign(&ana));
        assert!(r.body.starts_with("\n\n-- \nAna\n\nOn "), "{}", r.body);
        let mut r = Draft::reply(&m, &Body::default(), "ana@x.org", false);
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
        let mut reply = Draft::reply(&m, &body, "al@x.org", false);
        reply.quote_html(&m, "<p>Noon?</p>");
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

        let mut fwd = Draft::forward(&m, &body, Vec::new());
        fwd.quote_html(&m, "<p>Noon?</p>");
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
        new.quote_html(&m, "<p>Noon?</p>");
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
        assert!(err.to_string().contains("Komorebi's 100 MB"), "{err}");
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
        assert_eq!(errors[0], "folder: folders can't be attached");
        assert!(errors[1].starts_with("missing.pdf: "), "{}", errors[1]);
        // Counted against what is attached already.
        let (added, errors) = read_files(&paths[..1], MAX_ATTACH_HARD * 3 / 4);
        assert!(added.is_empty());
        assert!(errors[0].contains("100 MB"), "{}", errors[0]);
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
        assert_eq!(d.send_block(), "");
        d.attachments.push(file(3));
        assert_eq!(
            d.send_block(),
            "Attachments are over 25 MB; remove some to send."
        );
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

    struct TestPlatform(Rc<MinimalSoftwareWindow>);

    impl Platform for TestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
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
            slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
            let ui = AppWindow::new().unwrap();
            ui.window().set_size(slint::PhysicalSize::new(1280, 800));
            ui.show().unwrap();
            let path = std::env::temp_dir().join(format!(
                "komorebi-composer-{}-{name}.sqlite3",
                std::process::id()
            ));
            remove_db(&path);
            install(&ui, path.clone(), &|_| {}).unwrap();
            let mut db = Db::open(&path).unwrap();
            let server = |host: &str| Server {
                host: host.into(),
                port: 993,
                security: Security::Tls,
            };
            let account = db
                .add_account(&NewAccount {
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
            db.set_body(message, raw, "Noon?", false).unwrap();
            with_app(|a| {
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

    #[test]
    fn added_files_update_the_counter_and_are_saved() {
        let h = Harness::new("attach");
        // A drop with no composer open is ignored.
        h.app(|a| a.attach_dropped(vec![h.path.clone()]));
        assert!(!h.ui().get_composing());
        assert_eq!(h.status(), "");

        h.ui().invoke_compose();
        h.frame();
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
                vec!["x.txt: gone".into()],
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
    fn a_second_compose_keeps_the_first_and_switching_round_trips() {
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

        // A reply from the reader takes the composer; the draft is kept.
        let id = h.message;
        h.app(|a| a.select_message(id));
        assert!(!h.ui().get_composing());
        h.ui().invoke_reply();
        h.frame();
        assert_eq!(h.status(), "Earlier draft saved in Drafts.");
        assert_eq!(h.ui().get_c_title(), "Reply");
        assert_eq!(h.ui().get_c_to(), "bob@example.org");
        assert_eq!(h.ui().get_c_subject(), "Re: Lunch");
        assert_eq!(h.ui().get_c_cc(), "");
        assert!(!h.ui().get_c_show_cc());
        assert_eq!(h.ui().get_c_attachments().row_count(), 0);
        assert_eq!(h.ui().get_c_focus(), ComposeField::Body);
        assert_eq!(h.app(|a| a.drafts.len()), 2);

        // Esc on the untouched reply drops it; New mail brings the first
        // draft back, every field as typed.
        h.ui().invoke_c_close();
        h.frame();
        assert_eq!(h.app(|a| a.drafts.len()), 1);
        h.ui().invoke_compose();
        h.frame();
        assert!(h.ui().get_composing());
        assert_eq!(h.ui().get_c_title(), "New message");
        assert_eq!(h.ui().get_c_to(), typed[0].1);
        assert_eq!(h.ui().get_c_cc(), typed[1].1);
        assert_eq!(h.ui().get_c_bcc(), typed[2].1);
        assert_eq!(h.ui().get_c_subject(), typed[3].1);
        assert_eq!(h.ui().get_c_body(), typed[4].1);
        assert!(h.ui().get_c_show_cc());
        assert_eq!(h.ui().get_c_attachments().row_count(), 1);
        assert_eq!(h.ui().get_c_send_reason(), "");

        // New mail while it shows keeps it and starts another.
        h.ui().invoke_compose();
        h.frame();
        assert_eq!(h.ui().get_c_to(), "");
        assert_eq!(h.app(|a| a.drafts.len()), 2);
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

        // Alt+S sends (refused here) and types nothing.
        h.keys(&[Key::Alt], "s");
        assert_eq!(h.ui().get_c_to(), "a");
        assert_eq!(h.status(), "Fix the To address to send.");
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

        // Ctrl+Enter in a line field sends (refused: the Bcc error).
        h.app(|a| a.set_status("", false));
        h.keys(&[Key::Control], "\n");
        assert_eq!(h.status(), "Fix the Bcc address to send.");
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
        let body: String = h.ui().get_c_body().into();
        assert!(body.starts_with("Ok\n\nOn "), "{body}");
        assert!(body.ends_with("> Noon?"), "{body}");
        assert_eq!(h.app(|a| a.drafts.active().unwrap().body.clone()), body);
    }

    /// The formatting toolbar's spike, kept as its guard: the body's
    /// selection offsets (internal TextInput properties) are UTF-8 byte
    /// offsets after typing multi-byte text, selecting with the keyboard
    /// and undo, and a restored selection is exactly the one computed.
    #[test]
    fn formatting_keys_act_on_the_body_selection() {
        let h = Harness::new("format");
        let id = h.message;
        h.app(|a| a.select_message(id));
        h.ui().invoke_reply();
        h.frame();
        let first_line = || {
            let body = String::from(h.ui().get_c_body());
            assert_eq!(h.app(|a| a.drafts.active().unwrap().body.clone()), body);
            body.split('\n').next().unwrap_or_default().to_string()
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
        assert_eq!(first_line(), "ação **😀**");
        h.keys(&[ctrl], "b");
        assert_eq!(first_line(), "ação 😀");
        // The restored selection is the emoji: typing replaces it.
        h.keys(&[], "x");
        assert_eq!(first_line(), "ação x");
        // Ctrl+Z undoes the typing (the field's own undo), then the last
        // formatting edit (the field undoes the typing and the replaced
        // selection as two steps).
        h.keys(&[ctrl], "z");
        assert_eq!(first_line(), "ação ");
        h.keys(&[ctrl], "z");
        assert_eq!(first_line(), "ação 😀");
        h.keys(&[ctrl], "z");
        assert_eq!(first_line(), "ação **😀**");
        h.keys(&[ctrl], "z");
        assert_eq!(first_line(), "ação **😀**");

        // The whole line: italic around bold, then a list item.
        h.keys(&[], &key(Key::End));
        h.keys(&[shift], &key(Key::Home));
        h.keys(&[ctrl], "i");
        assert_eq!(first_line(), "_ação **😀**_");
        h.keys(&[ctrl, shift], "*");
        assert_eq!(first_line(), "- _ação **😀**_");
        // Shift+7 on a Portuguese layout.
        h.keys(&[ctrl, shift], "/");
        assert_eq!(first_line(), "1. _ação **😀**_");
        h.keys(&[ctrl, shift], "7");
        assert_eq!(first_line(), "_ação **😀**_");
        let html = mail::compose_html::to_html(&first_line());
        assert!(html.contains("<i>ação <b>😀</b></i>"), "{html}");

        // Ctrl+K asks for the address; Esc there closes only the field.
        h.keys(&[], &key(Key::End));
        h.keys(&[], " ");
        h.keys(&[ctrl], "k");
        h.keys(&[], "javascript:x");
        h.keys(&[], "\n");
        assert_eq!(first_line(), "_ação **😀**_ ");
        h.keys(&[], "\u{1b}");
        assert!(h.ui().get_composing());
        h.keys(&[], "see");
        assert_eq!(first_line(), "_ação **😀**_ see");
        h.keys(&[shift, ctrl], &key(Key::LeftArrow));
        h.keys(&[ctrl], "k");
        h.keys(&[], "example.org");
        h.keys(&[], "\n");
        assert_eq!(first_line(), "_ação **😀**_ [see](https://example.org)");
        // The caret is after the link, back in the body.
        h.keys(&[], ".");
        assert_eq!(first_line(), "_ação **😀**_ [see](https://example.org).");

        // Ctrl+Shift+B is still Bcc.
        h.keys(&[ctrl, shift], "B");
        assert!(h.ui().get_c_show_cc());
        let body: String = h.ui().get_c_body().into();
        assert!(body.ends_with("> Noon?"), "{body}");
    }

    #[test]
    fn an_html_reply_quotes_the_original_in_its_html_part() {
        let h = Harness::new("html-reply");
        let jobs = hold_builds(&h);
        let raw = b"From: Bob <bob@example.org>\r\nTo: me@example.org\r\nSubject: Lunch\r\n\
            Message-ID: <m1@example.org>\r\nMIME-Version: 1.0\r\n\
            Content-Type: text/html; charset=utf-8\r\n\r\n\
            <p style=\"color:#336\">Noon at <b>Rua Augusta</b>?</p>\
            <img src=\"https://t.example.invalid/p.gif\">\r\n";
        let mut db = Db::open(&h.path).unwrap();
        db.set_body(h.message, raw, "Noon", false).unwrap();
        let id = h.message;
        h.app(|a| a.select_message(id));
        h.ui().invoke_reply();
        h.frame();
        h.keys(&[], "O");
        h.keys(&[], "k");
        let quoted = h
            .app(|a| a.drafts.active().unwrap().quoted_html.clone())
            .expect("an HTML quote");
        assert!(quoted.contains("<b>Rua Augusta</b>"), "{quoted}");
        assert!(!quoted.contains("p.gif"), "{quoted}");

        h.ui().invoke_send();
        h.frame();
        let (row, _) = outbox_row(&h).expect("sent");
        // Saved with the draft, so Undo and a restart keep it.
        assert_eq!(db.draft(row).unwrap().unwrap().quoted_html, Some(quoted));
        assert_eq!(finish_build(&h, &jobs), Ok(true));
        let seq = h.app(|a| a.send_seq);
        h.app(|a| a.send_due(row, seq));
        let sent = db.claim_outbox(row).unwrap().expect("queued").raw;

        // The text part keeps the `> ` quote; the HTML part has the typed
        // text and the original's sanitized HTML instead.
        let text = mime::parse_body(&sent).text;
        assert!(text.starts_with("Ok\n\nOn "), "{text}");
        assert!(text.ends_with("> Noon at Rua Augusta?"), "{text}");
        let html = mime::html_body(&sent).expect("an HTML part");
        assert!(
            html.contains("<p>Ok</p><div class=\"komorebi_quote\" style=\"margin-top:1em\">On "),
            "{html}"
        );
        assert!(
            html.contains("<b>Rua Augusta</b>") && !html.contains("&gt; Noon"),
            "{html}"
        );
        assert!(!html.contains("p.gif"), "{html}");
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
        let mut reply = Draft::reply(&m, &Body::default(), "me@example.org", false);
        assert!(!reply.is_empty());
        assert_eq!(
            store(&c.db, &mut reply, "me@example.org").unwrap(),
            Stored::Nothing
        );
        assert_eq!(c.db.count_drafts().unwrap(), 0);
        assert!(d.row.is_none() && d.message_id.is_empty());
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
        // Arrow keys don't open drafts (their ids are not messages).
        h.ui().invoke_select_offset(1);
        h.frame();
        assert!(!h.ui().get_composing() && !h.ui().get_has_message());

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
        // The entry stays while its view is open, and goes after.
        assert!(nav(&h).contains(&(3, "Drafts".into(), 0)));
        h.ui().invoke_nav_clicked(NavItem::default());
        h.frame();
        assert!(!nav(&h).iter().any(|n| n.0 == 3));
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
        // Opening another message saves the reply.
        h.app(|a| a.select_message(id));
        assert!(!h.ui().get_composing());
        assert!(tagged(&h));
        let db = Db::open(&h.path).unwrap();
        let row = db.draft_for_message(id).unwrap().expect("saved");
        assert_eq!(db.count_drafts().unwrap(), 1);

        // After a restart, Reply (or Reply all) brings that draft back
        // instead of starting another.
        h.app(|a| a.drafts = Drafts::default());
        h.ui().invoke_reply_all();
        h.frame();
        assert!(h.ui().get_composing());
        assert_eq!(h.ui().get_c_title(), "Reply");
        let body: String = h.ui().get_c_body().into();
        assert!(body.starts_with("Ok\n\nOn "), "{body}");
        assert_eq!(h.app(|a| a.drafts.active().unwrap().row), Some(row));
        assert_eq!(db.count_drafts().unwrap(), 1);

        // Saved drafts are offered once at start.
        h.app(|a| {
            a.set_status("", false);
            a.offer_drafts();
        });
        assert_eq!(h.status(), "1 saved draft");
        assert_eq!(h.ui().get_status_action(), "Resume draft");
        // Another status line replaces the offer and its action.
        h.app(|a| a.set_status("Message sent.", false));
        assert_eq!(h.ui().get_status_action(), "");
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
        assert_eq!(h.status(), "Sending in 5 s");
        assert_eq!(h.ui().get_status_action(), "Undo");
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
        assert!(!nav.iter().any(|n| n.0 == 3), "no saved drafts left");
        // Frozen: no autosave writes it.
        h.app(|a| {
            assert!(a.drafts.active().is_none());
            a.autosave();
        });

        // Undo, before the build is done: the same draft comes back.
        h.ui().invoke_status_action_clicked();
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
        assert_eq!(h.ui().get_status_action(), "Undo");
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
        let failed = settled_row(&h, db::Outbox::Failed, "550 no such user");
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
        assert_eq!(r.preview, "Not sent: 550 no such user");
        h.ui().invoke_message_selected(r.id);
        h.frame();
        assert!(h.ui().get_o_show() && h.ui().get_o_problem());
        assert_eq!(h.ui().get_o_status(), "Not sent: 550 no such user");
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

        // Retrying a message that may have gone out asks first.
        let unconfirmed = settled_row(&h, db::Outbox::Unconfirmed, "may have been sent");
        h.ui().invoke_message_selected(unconfirmed as i32);
        h.frame();
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

        // Discard asks, then deletes it.
        h.ui().invoke_o_discard();
        h.frame();
        assert_eq!(h.ui().get_q_title(), "Discard this message?");
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
        assert_eq!(db.count_outbox().unwrap(), 0);
        assert_eq!(db.draft_ids("").unwrap(), [failed]);
        assert_eq!(h.ui().get_rows().row_count(), 0);
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

    /// The settings form as the window would send it for account `id`.
    fn edit_form(h: &Harness, id: i64) -> AccountForm {
        h.ui().invoke_edit_account(id as i32);
        h.frame();
        assert!(h.ui().get_show_settings());
        assert_eq!(h.ui().get_a_edit_id(), id as i32);
        h.ui().get_a_edit()
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
        let mut me = db.accounts().unwrap().remove(0);
        me.signature = "Me\nExample Org".into();
        db.update_account(&me).unwrap();
        let work = db
            .add_account(&NewAccount {
                email: "me@work.example".into(),
                name: "Me".into(),
                imap: me.imap.clone(),
                smtp: me.smtp.clone(),
                save_sent: true,
            })
            .unwrap();
        let mut w = db.accounts().unwrap().remove(1);
        assert_eq!(w.id, work);
        w.signature = "Me at Work".into();
        db.update_account(&w).unwrap();
        h.app(|a| a.load_accounts().unwrap());

        h.app(|a| a.compose_new());
        assert_eq!(h.ui().get_c_body(), "\n\n-- \nMe\nExample Org");
        // Only the signature: closing keeps nothing.
        h.ui().invoke_c_close();
        h.frame();
        assert_eq!(db.count_drafts().unwrap(), 0);

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
        let body = h.ui().get_c_body();
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
        let unread = || h.app(|a| a.nav.row_data(0).unwrap().unread);
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
        assert_eq!(h.status(), "Moved to Archive");
        assert_eq!(ui.get_status_action(), "Undo");
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
        ui.invoke_status_action_clicked();
        h.frame();
        assert!(listed(ids[1]));
        assert_eq!(open(), Some(ids[1]));
        assert!(h.app(|a| a.held.is_none()));
        assert_eq!(h.status(), "");
        assert_eq!(unread(), 4);

        // A selection of three, deleted: one label, one held batch.
        ui.invoke_message_extended(ids[3] as i32);
        h.frame();
        ui.invoke_delete();
        h.frame();
        assert_eq!(h.status(), "3 messages moved to Trash");
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
        assert_eq!(h.status(), "");
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
            "This account has no Junk folder, so nothing was moved."
        );
        assert!(h.ui().get_status_error());
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
        assert_eq!(h.status(), "Moved to Spam");
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
        assert!(ui.get_menu_junk() && ui.get_menu_unread());
        assert!(!ui.get_has_message());
        assert_eq!(ui.get_selected_count(), 1);
        assert!(!h.app(|a| a.db.message(spam).unwrap().unwrap().seen));
        // Not junk: read now, moved to the Inbox after the undo window.
        ui.invoke_junk(false);
        h.frame();
        assert_eq!(h.status(), "Moved to Inbox");
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
        // The third row of the list (rows are 76 px under a 100 px header).
        let position = slint::LogicalPosition::new(400.0, 100.0 + 76.0 * 2.5);
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
        assert!(ui.get_menu_unread());
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
        assert!(!ui.get_menu_unread());
        // On another row: that row alone, not opened, not read.
        assert!(ui.invoke_message_context(ids[2] as i32));
        h.frame();
        assert_eq!(open(), None);
        assert_eq!(ui.get_selected_count(), 1);
        assert!(ui.get_menu_unread() && ui.get_menu_unflagged() && !ui.get_menu_junk());
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
        // search box (where Insert and Ctrl+E don't reach the message).
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

        // Ctrl+Shift+U toggles read, Insert flags, Ctrl+E archives.
        assert!(message(ids[1]).seen);
        h.keys(&[Key::Control, Key::Shift], "u");
        assert!(!message(ids[1]).seen);
        h.keys(&[Key::Control, Key::Shift], "U");
        assert!(message(ids[1]).seen);
        h.keys(&[], &key(Key::Insert));
        assert!(message(ids[1]).flagged);
        h.keys(&[Key::Control], "e");
        assert_eq!(h.status(), "Moved to Archive");
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
        h.keys(&[Key::Control], "e");
        h.keys(&[Key::Control, Key::Shift], "u");
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

        // Unread: the four unread; a message read meanwhile stays listed
        // until the next reload.
        ui.invoke_set_filter(1);
        h.frame();
        assert_eq!(ui.get_list_filter(), 1);
        assert_eq!(listed(), 4);
        ui.invoke_message_selected(ids[2] as i32);
        h.frame();
        assert_eq!(listed(), 4);
        h.app(|a| a.reload_rows());
        assert_eq!(listed(), 3);
        // Flagged, then a search with no match.
        ui.invoke_set_filter(2);
        h.frame();
        assert_eq!(listed(), 1);
        h.app(|a| a.db.set_flagged(ids[3], false).unwrap());
        ui.invoke_set_filter(2);
        h.frame();
        assert_eq!(listed(), 0);
        assert_eq!(ui.get_list_empty(), "No flagged messages");
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
        h.app(|a| a.on_event(offline()));
        assert!(
            ui.get_status().contains("Can't reach the server"),
            "{}",
            ui.get_status()
        );
        h.app(|a| a.set_status("", false));
        // Again: quiet. After a sync that went through, it shows again.
        h.app(|a| a.on_event(offline()));
        assert_eq!(ui.get_status(), "");
        h.app(|a| {
            a.on_event(Event::Syncing {
                account_id: account,
                active: false,
                error: None,
            });
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
                    .map(|r| (r.sender.to_string(), r.folder.to_string()))
                    .collect::<Vec<_>>()
            })
        };

        // In the inbox, "joao" finds nothing; all folders find both, each
        // tagged with its folder and the sent one by recipient.
        ui.set_search_text("joao".into());
        ui.invoke_search_edited("joao".into());
        h.frame();
        assert!(rows().is_empty());
        assert_eq!(ui.get_list_empty(), "No matches");
        ui.invoke_set_search_all(true);
        h.frame();
        assert!(ui.get_search_all());
        assert_eq!(ui.get_list_title(), "All folders");
        assert_eq!(
            rows(),
            [
                (
                    "To: João Gonçalves <joao@example.org>".into(),
                    "Sent".into()
                ),
                ("João Gonçalves".into(), "Archive".into()),
            ]
        );
        // Typing on keeps the scope; folder actions keep to the view.
        ui.set_search_text("JOÃO GONÇ".into());
        ui.invoke_search_edited("JOÃO GONÇ".into());
        h.frame();
        assert_eq!(rows().len(), 2);
        ui.invoke_mark_all_read();
        h.frame();
        let archived = h.app(|a| a.db.message_ids(&[archive], "").unwrap());
        assert!(
            h.app(|a| a.db.briefs(&archived).unwrap())
                .iter()
                .all(|m| !m.seen)
        );
        // Cleared, the view lists as before, untagged.
        ui.invoke_clear_search();
        h.frame();
        assert_eq!(ui.get_list_title(), "All inboxes");
        assert_eq!(rows().len(), 6);
        assert!(rows().iter().all(|(_, folder)| folder.is_empty()));
        // Another view starts in its own folder again.
        h.app(|a| a.set_view(View::Folder(archive)));
        assert!(!ui.get_search_all());
        h.app(|a| a.set_view(View::Drafts));
        ui.invoke_set_search_all(true);
        h.frame();
        assert!(!ui.get_search_all());
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
                .set_body(id, raw, "", false)
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
            "Be careful with this sender: it failed your server's sender check (DMARC)."
        );
        assert!(ui.get_r_notice_warn() && ui.get_r_unsubscribe());
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
        assert!(!ui.get_r_notice_warn() && ui.get_r_unsubscribe());
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
            ui.get_r_notice()
                .starts_with("This message is encrypted (S/MIME). Komorebi cannot decrypt it;")
        );
        assert!(!ui.get_r_notice_warn() && !ui.get_r_unsubscribe());
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
                .set_body(id, raw.as_bytes(), "", false)
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
        assert_eq!(ints(ui.get_i_answers()), [1, 2, 3]);
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

        // Ctrl+U shows the header fields and the source; Esc closes it.
        h.keys(&[Key::Control], "u");
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
        // Ctrl+Shift+U still toggles read, and doesn't show the original.
        h.keys(&[Key::Control, Key::Shift], "U");
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
                error: "timed out".into(),
            });
            assert_eq!(a.raw_wait, None);
        });
        assert_eq!(h.status(), "Couldn't download this message: timed out");
        let raw = b"Subject: Report\r\n\r\nQ3\r\n";
        db.set_body(id, raw, "Q3", false).unwrap();
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
        // System, without a winit window here: the theme shown stays.
        ui.invoke_general_changed(0, 0);
        h.frame();
        assert_eq!(
            h.app(|a| a.db.settings().unwrap().theme),
            ThemeChoice::System
        );
        // Windows going dark is followed only under System.
        h.app(|a| a.system_theme(true));
        assert_eq!(
            ui.global::<crate::Palette>().get_color_scheme(),
            ColorScheme::Dark
        );
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
        ui.invoke_compose();
        h.frame();
        // To, Cc/Bcc, Subject, the formatting row, then the body.
        for _ in 0..4 {
            h.keys(&[], &key(Key::Tab));
        }
        h.keys(&[], "Z");
        assert_eq!(ui.get_c_body(), "Z");
        // Back on the row (Bold), Right reaches Italic.
        h.keys(&[Key::Shift], &key(Key::Tab));
        h.keys(&[], &key(Key::RightArrow));
        h.keys(&[], " ");
        assert!(ui.get_c_body().contains('_'), "{}", ui.get_c_body());
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
        // there for the next one.
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
        assert_eq!(ui.get_selected_folder(), 0);
        // The arrows stayed in the folders: no message was opened.
        assert!(!ui.get_has_message());
        // F6 back to the list, where Down opens the first message.
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

        // Settings start on the selected tab; Tab and Shift+Tab go round
        // inside the dialog, and Esc still closes it after many.
        ui.set_show_settings(true);
        h.frame();
        h.keys(&[], &key(Key::Tab));
        h.keys(&[], " ");
        assert_eq!(ui.get_settings_section(), 1);
        // Shift+Tab from the General tab: Accounts, Close, then round to
        // the section's last control (the taskbar flash).
        assert!(!h.app(|a| a.flash));
        for _ in 0..3 {
            h.keys(&[Key::Shift], &key(Key::Tab));
        }
        h.keys(&[], " ");
        assert!(h.app(|a| a.flash));
        for _ in 0..30 {
            h.keys(&[], &key(Key::Tab));
        }
        // A prompt over Settings gives the focus back to them.
        h.app(|a| a.ask(Question::Redownload(account)));
        h.keys(&[], &key(Key::Escape));
        assert_eq!(ui.get_q_title(), "");
        assert!(ui.get_show_settings());
        h.keys(&[], &key(Key::Escape));
        assert!(!ui.get_show_settings());
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

    #[test]
    fn escape_dismisses_the_status_line_last() {
        let h = Harness::new("toast-esc");
        let ui = h.ui();
        ui.invoke_focus_list();
        h.app(|a| a.set_status("Moved to Archive", false));
        h.keys(&[], &key(Key::Escape));
        assert_eq!(h.status(), "");
        // An offer stays: Esc would take the Undo away.
        h.app(|a| {
            a.set_status("Moved to Archive", false);
            a.set_status_action("Undo", StatusAction::None);
        });
        h.keys(&[], &key(Key::Escape));
        assert_eq!(h.status(), "Moved to Archive");
        // An open message closes first.
        let id = h.message;
        h.app(|a| {
            a.set_status("Saved", false);
            a.select_message(id);
        });
        h.keys(&[], &key(Key::Escape));
        assert!(!ui.get_has_message());
        assert_eq!(h.status(), "Saved");
        // Shift+F10 opens the row menu for what is selected (its labels
        // are set first).
        h.app(|a| a.select_message(id));
        ui.set_menu_unread(true);
        h.keys(&[Key::Shift], &key(Key::F10));
        assert!(!ui.get_menu_unread());
    }
}
