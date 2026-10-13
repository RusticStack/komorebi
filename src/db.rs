//! Local SQLite cache.
//!
//! Each thread opens its own connection: the UI thread reads, and the sync
//! workers write. WAL mode lets the reads continue during a sync. Multi-row
//! writes take the write lock up front (`BEGIN IMMEDIATE`), so a transaction
//! never fails halfway because another connection committed first.
//! Passwords are never stored here (see `secrets`).

use crate::fold::{fold as search_fold, fold_into, needs_fold};
use crate::mail::smtp;
use crate::providers::{Security, Server};
use crate::role::Role;
use anyhow::{Context, Result};
use rusqlite::functions::FunctionFlags;
use rusqlite::types::ValueRef;
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const SCHEMA_VERSION: i64 = 12;

/// How long a sync worker waits for another connection's write lock.
const WORKER_BUSY: Duration = Duration::from_secs(10);
/// The UI thread gives up sooner, so a long sync write never freezes it.
const UI_BUSY: Duration = Duration::from_secs(2);

/// Flag bits as packed by the sync (`mail::imap::flag_bits`).
pub const SEEN: u8 = 1;
pub const FLAGGED: u8 = 2;
pub const ANSWERED: u8 = 4;
/// Marked for removal on the server; such messages are hidden locally.
pub const DELETED: u8 = 8;

const SCHEMA: &str = r#"
CREATE TABLE accounts (
    id INTEGER PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL DEFAULT '',
    imap_host TEXT NOT NULL,
    imap_port INTEGER NOT NULL,
    imap_tls INTEGER NOT NULL,
    smtp_host TEXT NOT NULL,
    smtp_port INTEGER NOT NULL,
    smtp_tls INTEGER NOT NULL,
    save_sent INTEGER NOT NULL DEFAULT 1,
    position INTEGER NOT NULL DEFAULT 0,
    -- Plain text added below new mail, replies and forwards.
    signature TEXT NOT NULL DEFAULT '',
    -- The highest message id the address index has still to take in
    -- (`backfill_addresses`); NULL: none.
    addr_backfill INTEGER,
    -- 0: switched off in Settings (its cache stays; no connections).
    enabled INTEGER NOT NULL DEFAULT 1,
    -- Sign-in names; '' means the email address (`Account::imap_login`).
    imap_user TEXT NOT NULL DEFAULT '',
    smtp_user TEXT NOT NULL DEFAULT '',
    -- DER of the pinned server certificates (public data); NULL: none.
    imap_pin BLOB,
    smtp_pin BLOB
);

-- App settings by key; a missing key takes its default (`Settings`).
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE folders (
    id INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    display TEXT NOT NULL,
    depth INTEGER NOT NULL DEFAULT 0,
    role TEXT NOT NULL DEFAULT '',
    uidvalidity INTEGER NOT NULL DEFAULT 0,
    -- STATUS values at the last complete sync; NULL = never skip a pass.
    sync_uidnext INTEGER,
    sync_messages INTEGER,
    sync_hms INTEGER,
    -- A role the user chose for this folder; '' follows the detected `role`.
    role_override TEXT NOT NULL DEFAULT '',
    UNIQUE(account_id, name)
);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    uid INTEGER NOT NULL,
    msgid TEXT NOT NULL DEFAULT '',
    subject TEXT NOT NULL DEFAULT '',
    from_name TEXT NOT NULL DEFAULT '',
    from_addr TEXT NOT NULL DEFAULT '',
    to_addrs TEXT NOT NULL DEFAULT '',
    cc_addrs TEXT NOT NULL DEFAULT '',
    date INTEGER NOT NULL DEFAULT 0,
    size INTEGER NOT NULL DEFAULT 0,
    seen INTEGER NOT NULL DEFAULT 0,
    flagged INTEGER NOT NULL DEFAULT 0,
    answered INTEGER NOT NULL DEFAULT 0,
    has_attach INTEGER NOT NULL DEFAULT 0,
    preview TEXT NOT NULL DEFAULT '',
    has_body INTEGER NOT NULL DEFAULT 0,
    UNIQUE(folder_id, uid)
);
-- Matches the list's ORDER BY exactly, so a folder lists without a sort.
CREATE INDEX messages_folder_date ON messages(folder_id, date DESC, id DESC);
CREATE INDEX messages_folder_seen ON messages(folder_id, seen);

-- Raw messages live apart from the list rows, so scanning and searching the
-- list never pages through message bodies.
CREATE TABLE bodies (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    raw BLOB NOT NULL
);

-- Senders (exact lower-cased From address) whose remote images load
-- without asking.
CREATE TABLE remote_allow (
    address TEXT PRIMARY KEY,
    created INTEGER NOT NULL
);

-- Local drafts, saved as they are typed, and the outbox (`state` > 0, see
-- `Outbox`). Attachment bytes are files in the drafts folder beside the
-- cache (`drafts_dir`); only an outbox row's built message (`raw`, with the
-- attachments encoded in it) is stored here, until it is sent.
-- `quoted_html` is the sanitized original a reply or forward quotes in its
-- HTML part (`compose_html::attach`), or NULL. `rev` counts the changes and
-- `server_rev` is the one the copy in the server's Drafts folder holds
-- (`server_folder`, `server_uidvalidity`, `server_uid`; NULL: none).
CREATE TABLE drafts (
    id INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    kind INTEGER NOT NULL DEFAULT 0,
    -- The message replied to; message ids can be reused, so it is cleared
    -- when that message leaves the cache.
    reply_to_msg INTEGER REFERENCES messages(id) ON DELETE SET NULL,
    in_reply_to TEXT NOT NULL DEFAULT '',
    refs TEXT NOT NULL DEFAULT '',
    to_addrs TEXT NOT NULL DEFAULT '',
    cc_addrs TEXT NOT NULL DEFAULT '',
    bcc_addrs TEXT NOT NULL DEFAULT '',
    subject TEXT NOT NULL DEFAULT '',
    body TEXT NOT NULL DEFAULT '',
    quoted_html TEXT,
    message_id TEXT NOT NULL,
    state INTEGER NOT NULL DEFAULT 0,
    created INTEGER NOT NULL,
    updated INTEGER NOT NULL,
    raw BLOB,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_try INTEGER,
    last_error TEXT,
    rev INTEGER NOT NULL DEFAULT 1,
    server_rev INTEGER NOT NULL DEFAULT 0,
    server_folder INTEGER REFERENCES folders(id) ON DELETE SET NULL,
    server_uidvalidity INTEGER,
    server_uid INTEGER
);
CREATE INDEX drafts_state ON drafts(state, updated DESC, id DESC);
CREATE INDEX drafts_reply ON drafts(reply_to_msg);

-- `file` names the copy in the draft's folder (the row id).
CREATE TABLE draft_attachments (
    id INTEGER PRIMARY KEY,
    draft_id INTEGER NOT NULL REFERENCES drafts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    mime TEXT NOT NULL DEFAULT '',
    size INTEGER NOT NULL,
    file TEXT NOT NULL
);
CREATE INDEX draft_attachments_draft ON draft_attachments(draft_id);
"#;

/// The address index for recipient suggestions: derived from the cached
/// headers (`index_addresses`), never edited, so it can be rebuilt from
/// the messages at any time. `addr` is lower-cased; `folded` is the
/// display name lower-cased, for matching.
const ADDRESSES: &str = r#"
CREATE TABLE addresses (
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    addr TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    folded TEXT NOT NULL DEFAULT '',
    last_seen INTEGER NOT NULL DEFAULT 0,
    weight INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, addr)
);
CREATE INDEX addresses_folded ON addresses(account_id, folded);
"#;

/// v10: the answers given to meeting invitations, one per event (and
/// occurrence: `recurrence_id` as the invite writes it, empty for the
/// series), so a reopened invite shows the answer. `sequence` is the
/// invite's SEQUENCE when answered (a later update asks again), `partstat`
/// the answer (`ical::Answer`) and `answered` unix seconds. No reply text
/// or attendee list is kept.
const RSVP: &str = r#"
CREATE TABLE rsvp (
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    uid TEXT NOT NULL,
    recurrence_id TEXT NOT NULL DEFAULT '',
    sequence INTEGER,
    partstat INTEGER,
    answered INTEGER,
    PRIMARY KEY (account_id, uid, recurrence_id)
);
"#;

/// v1 kept raw bodies inline in `messages`.
const MIGRATE_V1_TO_V2: &str = r#"
CREATE TABLE bodies (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    raw BLOB NOT NULL
);
INSERT INTO bodies (message_id, raw) SELECT id, raw FROM messages WHERE raw IS NOT NULL;
ALTER TABLE messages ADD COLUMN has_body INTEGER NOT NULL DEFAULT 0;
UPDATE messages SET has_body = 1 WHERE raw IS NOT NULL;
ALTER TABLE messages DROP COLUMN raw;
DROP INDEX messages_folder_date;
CREATE INDEX messages_folder_date ON messages(folder_id, date DESC, id DESC);
"#;

/// v3 adds the folder sync fingerprint (all NULL until the next full sync).
const MIGRATE_V2_TO_V3: &str = r#"
ALTER TABLE folders ADD COLUMN sync_uidnext INTEGER;
ALTER TABLE folders ADD COLUMN sync_messages INTEGER;
-- HIGHESTMODSEQ fits in i64 (RFC 7162: < 2^63).
ALTER TABLE folders ADD COLUMN sync_hms INTEGER;
"#;

/// v4 adds the senders whose remote images load without asking.
const MIGRATE_V3_TO_V4: &str = r#"
CREATE TABLE remote_allow (
    address TEXT PRIMARY KEY,
    created INTEGER NOT NULL
);
"#;

/// v5 adds local drafts.
const MIGRATE_V4_TO_V5: &str = r#"
-- Local drafts, saved as they are typed. Attachment bytes are files in the
-- drafts folder beside the cache (`drafts_dir`), never stored here.
-- `state` 0 is a draft; `quoted_html` is reserved (NULL).
CREATE TABLE drafts (
    id INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    kind INTEGER NOT NULL DEFAULT 0,
    -- The message replied to; message ids can be reused, so it is cleared
    -- when that message leaves the cache.
    reply_to_msg INTEGER REFERENCES messages(id) ON DELETE SET NULL,
    in_reply_to TEXT NOT NULL DEFAULT '',
    refs TEXT NOT NULL DEFAULT '',
    to_addrs TEXT NOT NULL DEFAULT '',
    cc_addrs TEXT NOT NULL DEFAULT '',
    bcc_addrs TEXT NOT NULL DEFAULT '',
    subject TEXT NOT NULL DEFAULT '',
    body TEXT NOT NULL DEFAULT '',
    quoted_html TEXT,
    message_id TEXT NOT NULL,
    state INTEGER NOT NULL DEFAULT 0,
    created INTEGER NOT NULL,
    updated INTEGER NOT NULL
);
CREATE INDEX drafts_state ON drafts(state, updated DESC, id DESC);
CREATE INDEX drafts_reply ON drafts(reply_to_msg);

-- `file` names the copy in the draft's folder (the row id).
CREATE TABLE draft_attachments (
    id INTEGER PRIMARY KEY,
    draft_id INTEGER NOT NULL REFERENCES drafts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    mime TEXT NOT NULL DEFAULT '',
    size INTEGER NOT NULL,
    file TEXT NOT NULL
);
CREATE INDEX draft_attachments_draft ON draft_attachments(draft_id);
"#;

/// v6 turns drafts with `state` > 0 into the outbox.
const MIGRATE_V5_TO_V6: &str = r#"
ALTER TABLE drafts ADD COLUMN raw BLOB;
ALTER TABLE drafts ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE drafts ADD COLUMN next_try INTEGER;
ALTER TABLE drafts ADD COLUMN last_error TEXT;
"#;

/// v7 adds account signatures and the settings table.
const MIGRATE_V6_TO_V7: &str = r#"
ALTER TABLE accounts ADD COLUMN signature TEXT NOT NULL DEFAULT '';
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

/// v8 adds the address index (`ADDRESSES`, created with this step). The
/// messages already cached are indexed later by the sync worker
/// (`backfill_addresses`), newest first; mail stored from now on is
/// indexed as it arrives, with ids above the mark.
const MIGRATE_V7_TO_V8: &str = r#"
ALTER TABLE accounts ADD COLUMN addr_backfill INTEGER;
UPDATE accounts SET addr_backfill = (SELECT MAX(id) FROM messages WHERE account_id = accounts.id);
"#;

/// v9 tracks each draft's copy in the server's Drafts folder. Drafts
/// saved before count as changed, so the next sync copies them there.
const MIGRATE_V8_TO_V9: &str = r#"
ALTER TABLE drafts ADD COLUMN rev INTEGER NOT NULL DEFAULT 1;
ALTER TABLE drafts ADD COLUMN server_rev INTEGER NOT NULL DEFAULT 0;
ALTER TABLE drafts ADD COLUMN server_folder INTEGER REFERENCES folders(id) ON DELETE SET NULL;
ALTER TABLE drafts ADD COLUMN server_uidvalidity INTEGER;
ALTER TABLE drafts ADD COLUMN server_uid INTEGER;
"#;

/// v11 lets an account be switched off.
const MIGRATE_V10_TO_V11: &str = r#"
ALTER TABLE accounts ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1;
"#;

/// v12 adds separate sign-in names and pinned certificates per account,
/// and a per-folder role the user chose (all unused columns default to
/// "same as before"; one bump for the whole self-hosting work).
const MIGRATE_V11_TO_V12: &str = r#"
ALTER TABLE accounts ADD COLUMN imap_user TEXT NOT NULL DEFAULT '';
ALTER TABLE accounts ADD COLUMN smtp_user TEXT NOT NULL DEFAULT '';
ALTER TABLE accounts ADD COLUMN imap_pin BLOB;
ALTER TABLE accounts ADD COLUMN smtp_pin BLOB;
ALTER TABLE folders ADD COLUMN role_override TEXT NOT NULL DEFAULT '';
"#;

/// The cache was made by a newer Komorebi (its schema version is higher):
/// it is left as it is, never deleted or migrated down.
#[derive(Debug)]
pub struct NewerCache(pub i64);

impl std::fmt::Display for NewerCache {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "This cache was made by a newer Komorebi (schema v{}; this version reads v{SCHEMA_VERSION}). \
             Update Komorebi to open it; the file was left as it is.",
            self.0
        )
    }
}

impl std::error::Error for NewerCache {}

/// SQLite says the file is damaged: corrupt, or not a database.
fn damaged(e: &anyhow::Error) -> bool {
    use rusqlite::ffi::ErrorCode;
    e.chain().any(|c| {
        matches!(
            c.downcast_ref::<rusqlite::Error>(),
            Some(rusqlite::Error::SqliteFailure(f, _))
                if matches!(f.code, ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase)
        )
    })
}

/// Rename a damaged cache aside, dated (`cache.damaged-20261009-101500.sqlite3`),
/// with its `-wal` and `-shm` (left behind, they would be replayed into the
/// new cache) and its drafts folder (whose numbered folders the new
/// drafts would reuse). Returns the new name.
fn set_aside(path: &Path, now: &chrono::DateTime<chrono::Local>) -> Result<PathBuf> {
    let name = |suffix: &str| PathBuf::from(format!("{}{suffix}", path.display()));
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path.extension().unwrap_or_default().to_string_lossy();
    let aside = path.with_file_name(format!(
        "{stem}.damaged-{}.{ext}",
        now.format("%Y%m%d-%H%M%S")
    ));
    let aside_name = |suffix: &str| PathBuf::from(format!("{}{suffix}", aside.display()));
    for suffix in ["-wal", "-shm"] {
        if name(suffix).exists() {
            std::fs::rename(name(suffix), aside_name(suffix))
                .with_context(|| format!("setting the damaged cache's {suffix} file aside"))?;
        }
    }
    std::fs::rename(path, &aside).context("setting the damaged cache aside")?;
    let drafts = drafts_dir(path);
    if drafts.exists() {
        std::fs::rename(&drafts, drafts_dir(&aside)).ok();
    }
    Ok(aside)
}

/// `%LOCALAPPDATA%\komorebi\cache.sqlite3` (local, not roaming: it holds
/// plaintext mail), or `KOMOREBI_DB` if set (a throwaway cache for testing).
pub fn default_path() -> PathBuf {
    if let Some(p) = crate::dev_var("KOMOREBI_DB") {
        return PathBuf::from(p);
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("komorebi")
        .join("cache.sqlite3")
}

#[derive(Debug, Clone)]
pub struct Account {
    pub id: i64,
    pub email: String,
    pub name: String,
    pub imap: Server,
    pub smtp: Server,
    pub save_sent: bool,
    /// Plain text, without the `-- ` delimiter line.
    pub signature: String,
    /// False: switched off in Settings (no workers, no connections).
    pub enabled: bool,
    /// IMAP sign-in name; empty: the address (`imap_login`).
    pub imap_user: String,
    /// SMTP sign-in name; empty: the address (`smtp_login`).
    pub smtp_user: String,
    /// DER of the pinned certificate of the IMAP / SMTP server.
    pub imap_pin: Option<Vec<u8>>,
    pub smtp_pin: Option<Vec<u8>>,
}

impl Account {
    /// The name to sign in to the IMAP server with.
    pub fn imap_login(&self) -> &str {
        login_or(&self.imap_user, &self.email)
    }

    /// The name to sign in to the SMTP server with.
    pub fn smtp_login(&self) -> &str {
        login_or(&self.smtp_user, &self.email)
    }
}

/// `user`, or `email` when no separate sign-in name is set.
pub fn login_or<'a>(user: &'a str, email: &'a str) -> &'a str {
    if user.is_empty() { email } else { user }
}

/// A new account; its signature starts empty (set with `update_account`).
#[derive(Debug, Clone)]
pub struct NewAccount {
    pub email: String,
    pub name: String,
    pub imap: Server,
    pub smtp: Server,
    pub save_sent: bool,
    pub imap_user: String,
    pub smtp_user: String,
}

#[derive(Debug, Clone)]
pub struct Folder {
    pub id: i64,
    pub account_id: i64,
    pub name: String,
    pub display: String,
    pub depth: i64,
    pub role: Role,
    /// Last UIDVALIDITY seen by a sync (0: never synced).
    pub uidvalidity: u32,
}

/// A folder's STATUS (UIDNEXT, MESSAGES, HIGHESTMODSEQ) as of its last
/// complete sync. While the server reports the same values, nothing changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fp {
    pub uidnext: u32,
    pub messages: u32,
    pub hms: u64,
}

/// A folder's role as the app uses it: the one the user chose
/// (`role_override`) or else the one detected from the server.
const ROLE_SQL: &str = "CASE WHEN role_override <> '' THEN role_override ELSE role END";

const FOLDER_COLUMNS: &str = "id, account_id, name, display, depth,
     CASE WHEN role_override <> '' THEN role_override ELSE role END, uidvalidity";

impl Folder {
    fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Folder {
            id: r.get(0)?,
            account_id: r.get(1)?,
            name: r.get(2)?,
            display: r.get(3)?,
            depth: r.get(4)?,
            role: Role::from_key(&r.get::<_, String>(5)?),
            uidvalidity: r.get(6)?,
        })
    }
}

/// One row of the message list. Deliberately excludes bodies.
#[derive(Debug, Clone)]
pub struct Header {
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub to: String,
    pub date: i64,
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    pub has_attach: bool,
    pub preview: String,
    /// A local draft replies to this message.
    pub has_draft: bool,
    pub folder_id: i64,
    pub account_id: i64,
}

/// Header fields captured during sync, before a row exists.
#[derive(Debug, Clone, Default)]
pub struct NewHeader {
    pub uid: u32,
    pub msgid: String,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub to: String,
    pub cc: String,
    pub date: i64,
    pub size: i64,
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    pub has_attach: bool,
}

/// Everything the reader and composer need about one message.
#[derive(Debug, Clone)]
pub struct Message {
    pub id: i64,
    pub account_id: i64,
    pub folder_id: i64,
    pub uid: u32,
    pub msgid: String,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub to: String,
    pub cc: String,
    pub date: i64,
    pub seen: bool,
    pub flagged: bool,
    pub raw: Option<Vec<u8>>,
}

/// What a list action needs about one message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Brief {
    pub id: i64,
    pub account_id: i64,
    pub folder_id: i64,
    pub uid: u32,
    pub seen: bool,
    pub flagged: bool,
}

impl From<&Message> for Brief {
    fn from(m: &Message) -> Self {
        Self {
            id: m.id,
            account_id: m.account_id,
            folder_id: m.folder_id,
            uid: m.uid,
            seen: m.seen,
            flagged: m.flagged,
        }
    }
}

/// A suggested recipient from the address index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipient {
    /// The newest display name seen with the address (may be empty).
    pub name: String,
    /// Lower-cased.
    pub addr: String,
}

/// What a message adds to an address's weight in the index: once as its
/// sender, more as a recipient of mail the user sent.
const FROM_WEIGHT: i64 = 1;
const SENT_WEIGHT: i64 = 3;

/// Cached messages the address backfill takes in per transaction.
pub const BACKFILL_CHUNK: usize = 5_000;

const UPSERT_ADDRESS: &str =
    "INSERT INTO addresses (account_id, addr, name, folded, last_seen, weight)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
     ON CONFLICT (account_id, addr) DO UPDATE SET
        weight = weight + excluded.weight,
        last_seen = max(last_seen, excluded.last_seen),
        name = CASE WHEN excluded.name <> '' AND (name = '' OR excluded.last_seen >= last_seen)
                    THEN excluded.name ELSE name END,
        folded = CASE WHEN excluded.name <> '' AND (name = '' OR excluded.last_seen >= last_seen)
                      THEN excluded.folded ELSE folded END";

/// App settings (the `settings` table). A key that is missing, unknown or
/// unreadable leaves its default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Seconds Send waits so it can be undone (key `undo_send_secs`).
    pub undo_send_secs: u64,
    /// When an opened message is marked read (key `mark_read_delay_ms`).
    pub mark_read: MarkRead,
    /// The app theme (key `theme`: `system`, `light` or `dark`).
    pub theme: ThemeChoice,
    /// Seconds between new-mail checks (key `check_interval_secs`: one
    /// of [`CHECK_INTERVALS`]).
    pub check_interval_secs: u64,
    /// Flash the taskbar button when new mail arrives while the window
    /// is in the background (key `flash_taskbar`: `1` or `0`).
    pub flash_taskbar: bool,
    /// The window's place when it last closed (key `window`).
    pub window: Option<Geometry>,
    /// Keep the diagnostics log (`crate::diag`; key `diagnostics_log`:
    /// `1` or `0`). Off by default.
    pub diagnostics_log: bool,
    /// The display language (key `language`: `system`, `en` or `pt-PT`).
    pub language: crate::i18n::LangSetting,
    /// The folder pane's folded accounts and folders (key `nav_folded`:
    /// `a<account id>` and `f<folder id>` tokens, comma separated; see
    /// [`crate::tree::parse_folds`]).
    pub nav_folded: String,
    /// The view the window showed when it last closed (key `last_view`:
    /// `all`, `drafts`, `outbox` or `f<folder id>`; empty: none saved).
    pub last_view: String,
    /// The accounts whose folders have had their default folds (key
    /// `nav_folds_init`: account ids, comma separated; see
    /// [`crate::tree::default_folds`]).
    pub nav_folds_init: String,
    /// The folders Move went to last, the last first (key `move_recent`:
    /// folder ids, comma separated).
    pub move_recent: String,
    /// The teaching tips already seen (key `tips_seen`: `folders`,
    /// `search`, `help`, comma separated; see [`crate::onboard::Seen`]).
    pub tips_seen: String,
    /// The version the What's new bar last told about (key
    /// `whats_new_version`; `None`: never, a version from before the bar).
    pub whats_new_version: Option<String>,
    /// The composer's one-time caption about the formatting marks was
    /// shown (key `format_hint_seen`).
    pub format_hint_seen: bool,
}

/// The new-mail check intervals Settings offers, in seconds.
pub const CHECK_INTERVALS: [u64; 4] = [60, 120, 300, 900];

/// The app theme: as Windows, or always light or dark.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub fn key(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        }
    }
}

/// A window's outer position and size in physical pixels, and whether it
/// was maximized (then the rest is where it goes when restored).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub maximized: bool,
}

impl Geometry {
    /// "x y w h max" (`max` 1 or 0), as the setting stores it.
    pub fn to_setting(self) -> String {
        let max = u8::from(self.maximized);
        format!("{} {} {} {} {max}", self.x, self.y, self.w, self.h)
    }

    /// The setting back; `None` unless it has five numbers and a size of
    /// at least 1x1.
    pub fn from_setting(value: &str) -> Option<Self> {
        let mut it = value.split_whitespace().map(|n| n.parse::<i32>().ok());
        let mut next = || it.next().flatten();
        let g = Geometry {
            x: next()?,
            y: next()?,
            w: next()?,
            h: next()?,
            maximized: next()? != 0,
        };
        (g.w > 0 && g.h > 0 && it.next().is_none()).then_some(g)
    }
}

/// The longest undo-send delay a saved setting may ask for.
pub const MAX_UNDO_SEND_SECS: u64 = 60;
/// The longest mark-as-read delay a saved setting may ask for.
pub const MAX_MARK_READ_DELAY_MS: u64 = 60_000;

/// When an opened message is marked read: `mark_read_delay_ms` 0 is at
/// once, -1 (any negative value) never, else after that long open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkRead {
    Now,
    After(Duration),
    Never,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            undo_send_secs: 5,
            mark_read: MarkRead::After(Duration::from_secs(1)),
            theme: ThemeChoice::System,
            check_interval_secs: CHECK_INTERVALS[0],
            flash_taskbar: false,
            window: None,
            diagnostics_log: false,
            language: crate::i18n::LangSetting::System,
            nav_folded: String::new(),
            last_view: String::new(),
            nav_folds_init: String::new(),
            move_recent: String::new(),
            tips_seen: String::new(),
            whats_new_version: None,
            format_hint_seen: false,
        }
    }
}

impl Settings {
    fn apply(&mut self, key: &str, value: &str) {
        match key {
            "undo_send_secs" => {
                if let Ok(secs) = value.trim().parse::<u64>() {
                    self.undo_send_secs = secs.min(MAX_UNDO_SEND_SECS);
                }
            }
            "mark_read_delay_ms" => match value.trim().parse::<i64>() {
                Ok(0) => self.mark_read = MarkRead::Now,
                Ok(ms) if ms < 0 => self.mark_read = MarkRead::Never,
                Ok(ms) => {
                    let ms = ms.unsigned_abs().min(MAX_MARK_READ_DELAY_MS);
                    self.mark_read = MarkRead::After(Duration::from_millis(ms));
                }
                Err(_) => {}
            },
            "theme" => match value.trim() {
                "system" => self.theme = ThemeChoice::System,
                "light" => self.theme = ThemeChoice::Light,
                "dark" => self.theme = ThemeChoice::Dark,
                _ => {}
            },
            "check_interval_secs" => {
                if let Ok(secs) = value.trim().parse::<u64>()
                    && CHECK_INTERVALS.contains(&secs)
                {
                    self.check_interval_secs = secs;
                }
            }
            "flash_taskbar" => match value.trim() {
                "1" => self.flash_taskbar = true,
                "0" => self.flash_taskbar = false,
                _ => {}
            },
            "window" => {
                if let Some(g) = Geometry::from_setting(value) {
                    self.window = Some(g);
                }
            }
            "diagnostics_log" => match value.trim() {
                "1" => self.diagnostics_log = true,
                "0" => self.diagnostics_log = false,
                _ => {}
            },
            "language" => {
                if let Some(language) = crate::i18n::LangSetting::from_key(value) {
                    self.language = language;
                }
            }
            "nav_folded" => self.nav_folded = value.to_string(),
            "last_view" => self.last_view = value.trim().to_string(),
            "nav_folds_init" => self.nav_folds_init = value.to_string(),
            "move_recent" => self.move_recent = value.to_string(),
            "tips_seen" => self.tips_seen = value.to_string(),
            "whats_new_version" => self.whats_new_version = Some(value.trim().to_string()),
            "format_hint_seen" => self.format_hint_seen = value == "1",
            _ => {}
        }
    }
}

/// Which messages the list shows (the Unread and Flagged toggles).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Unread,
    Flagged,
}

/// Where a cache keeps its drafts' attachment copies: `cache.drafts`
/// beside `cache.sqlite3` (each cache file gets its own folder, so
/// throwaway caches side by side never share one).
pub fn drafts_dir(db_path: &Path) -> PathBuf {
    db_path.with_extension("drafts")
}

/// A local draft as saved. `id` is 0 until the first save.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DraftRow {
    pub id: i64,
    pub account_id: i64,
    /// 0 new, 1 reply, 2 reply all, 3 forward.
    pub kind: i64,
    pub reply_to_msg: Option<i64>,
    pub in_reply_to: String,
    pub refs: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
    /// The HTML quote of a reply or forward (`compose_html::attach`).
    pub quoted_html: Option<String>,
    pub message_id: String,
    /// Unix seconds of the last save.
    pub updated: i64,
}

/// A draft's copy in the server's Drafts folder of account `account_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerCopy {
    pub account_id: i64,
    pub folder_id: i64,
    /// The folder's UIDVALIDITY when the copy was saved: a later one means
    /// `uid` may name another message, so the copy is left alone.
    pub uidvalidity: u32,
    pub uid: u32,
}

/// What recording a draft's new server copy found.
#[derive(Debug, PartialEq, Eq)]
pub enum Recorded {
    /// Recorded; the copy it replaces, if there was one.
    Replaced(Option<ServerCopy>),
    /// The draft is gone, or is no longer a draft (sent, or in the outbox).
    NotADraft,
}

/// The server copy of draft `id`, if it has one in a folder still cached.
fn server_copy(conn: &Connection, id: i64) -> rusqlite::Result<Option<ServerCopy>> {
    conn.prepare_cached(
        "SELECT f.account_id, f.id, d.server_uidvalidity, d.server_uid
         FROM drafts d JOIN folders f ON f.id = d.server_folder
         WHERE d.id = ?1 AND d.server_uid IS NOT NULL AND d.server_uidvalidity IS NOT NULL",
    )?
    .query_row([id], |r| {
        Ok(ServerCopy {
            account_id: r.get(0)?,
            folder_id: r.get(1)?,
            uidvalidity: r.get(2)?,
            uid: r.get(3)?,
        })
    })
    .optional()
}

/// An attachment of a saved draft; its bytes are in the drafts folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftFile {
    pub id: i64,
    pub name: String,
    pub mime: String,
    pub size: i64,
}

/// One row of the Drafts view.
#[derive(Debug, Clone)]
pub struct DraftHeader {
    pub to: String,
    pub subject: String,
    /// The start of the body.
    pub preview: String,
    pub updated: i64,
    pub has_attach: bool,
}

/// Where an outbox row is: its `drafts.state` (0 is a draft).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outbox {
    /// Send was pressed and the undo window runs; the message is built.
    Pending = 1,
    /// Built, waiting for a send (or for its retry time).
    Queued = 2,
    /// A send has it.
    Sending = 3,
    /// Not sent: refused, or out of retries.
    Failed = 4,
    /// The connection broke during the send, so it may have gone out.
    /// Never sent again without the user.
    Unconfirmed = 5,
}

impl Outbox {
    fn from_code(code: i64) -> Option<Self> {
        Some(match code {
            1 => Outbox::Pending,
            2 => Outbox::Queued,
            3 => Outbox::Sending,
            4 => Outbox::Failed,
            5 => Outbox::Unconfirmed,
            _ => return None,
        })
    }
}

/// What a send needs from the outbox row it took.
#[derive(Debug, Clone)]
pub struct Outbound {
    pub to: String,
    pub cc: String,
    pub bcc: String,
    /// The built message, sent and saved to Sent byte for byte.
    pub raw: Vec<u8>,
    pub reply_to_msg: Option<i64>,
    /// Sends tried, this one included.
    pub attempts: i64,
}

/// `drafts.kind` of an invitation's reply ([`Db::queue_reply`]): an
/// outbox row only, never edited.
pub const INVITE_REPLY: i64 = 4;

/// One outbox row, for its list row and details.
#[derive(Debug, Clone)]
pub struct OutboxEntry {
    pub account_id: i64,
    pub to: String,
    pub subject: String,
    pub state: Outbox,
    /// Unix seconds of the next automatic try (queued rows only).
    pub next_try: Option<i64>,
    pub last_error: String,
    pub updated: i64,
    pub has_attach: bool,
    /// An invitation's reply: it can't go back to Drafts.
    pub invite_reply: bool,
    /// The text as typed (its first lines show in the Outbox).
    pub body: String,
}

const DRAFT_COLUMNS: &str = "id, account_id, kind, reply_to_msg, in_reply_to, refs, to_addrs,     cc_addrs, bcc_addrs, subject, body, message_id, updated, quoted_html";

impl DraftRow {
    fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(DraftRow {
            id: r.get(0)?,
            account_id: r.get(1)?,
            kind: r.get(2)?,
            reply_to_msg: r.get(3)?,
            in_reply_to: r.get(4)?,
            refs: r.get(5)?,
            to: r.get(6)?,
            cc: r.get(7)?,
            bcc: r.get(8)?,
            subject: r.get(9)?,
            body: r.get(10)?,
            message_id: r.get(11)?,
            updated: r.get(12)?,
            quoted_html: r.get(13)?,
        })
    }
}

pub struct Db {
    conn: Connection,
    /// `drafts_dir` of the cache file (none for an in-memory cache).
    drafts: Option<PathBuf>,
}

impl Db {
    /// Open the cache, creating or upgrading it as needed.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        let mut db = Self::init(conn)?;
        db.drafts = Some(drafts_dir(path));
        Ok(db)
    }

    /// The UI thread's connection. It migrates the cache before any worker
    /// starts, then waits only briefly for the write lock.
    ///
    /// A cache SQLite reports as damaged (corrupt, or not a database) is
    /// renamed aside with its `-wal`, `-shm` and drafts folder, and a new
    /// one created; the aside file's path comes back. Its accounts are lost
    /// with it (they live in the cache). A cache made by a newer Komorebi
    /// is refused ([`NewerCache`]) and left alone.
    pub fn open_ui(path: &Path) -> Result<(Self, Option<PathBuf>)> {
        let opened = Self::open(path).and_then(|db| {
            // Reads the schema page, which a damaged file often fails.
            db.conn
                .query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
            Ok(db)
        });
        let (db, aside) = match opened {
            Ok(db) => (db, None),
            Err(e) if damaged(&e) => {
                let aside = set_aside(path, &chrono::Local::now())?;
                (Self::open(path)?, Some(aside))
            }
            Err(e) => return Err(e),
        };
        db.conn.busy_timeout(UI_BUSY)?;
        Ok((db, aside))
    }

    /// A sync worker's connection. Workers never create or migrate the
    /// cache, so they refuse a file at any other schema version.
    pub fn open_worker(path: &Path) -> Result<Self> {
        let flags = OpenFlags::default() - OpenFlags::SQLITE_OPEN_CREATE;
        let conn = Connection::open_with_flags(path, flags)
            .with_context(|| format!("opening {}", path.display()))?;
        configure(&conn)?;
        let version = user_version(&conn)?;
        if version != SCHEMA_VERSION {
            anyhow::bail!("cache schema v{version} is not v{SCHEMA_VERSION}; restart the app");
        }
        // A delivered send deletes its outbox row and the attachment copies.
        Ok(Self {
            conn,
            drafts: Some(drafts_dir(path)),
        })
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        configure(&conn)?;
        let mut version = user_version(&conn)?;
        if version > SCHEMA_VERSION {
            return Err(NewerCache(version).into());
        }
        if version == 0 {
            conn.execute_batch(&format!(
                "BEGIN; {SCHEMA} {ADDRESSES} {RSVP} PRAGMA user_version = {SCHEMA_VERSION}; COMMIT;"
            ))
            .context("creating cache schema")?;
            return Ok(Self { conn, drafts: None });
        }
        if version == 1 {
            conn.execute_batch(&format!(
                "BEGIN; {MIGRATE_V1_TO_V2} PRAGMA user_version = 2; COMMIT;"
            ))
            .context("upgrading cache schema")?;
            // Reclaim the space the inline bodies used (needs no transaction).
            conn.execute_batch("VACUUM;").ok();
            version = 2;
        }
        let v8 = format!("{MIGRATE_V7_TO_V8} {ADDRESSES}");
        for (from, step) in [
            (2, MIGRATE_V2_TO_V3),
            (3, MIGRATE_V3_TO_V4),
            (4, MIGRATE_V4_TO_V5),
            (5, MIGRATE_V5_TO_V6),
            (6, MIGRATE_V6_TO_V7),
            (7, v8.as_str()),
            (8, MIGRATE_V8_TO_V9),
            (9, RSVP),
            (10, MIGRATE_V10_TO_V11),
            (11, MIGRATE_V11_TO_V12),
        ] {
            if version != from {
                continue;
            }
            conn.execute_batch("BEGIN IMMEDIATE;")
                .context("upgrading cache schema")?;
            // Another process may have migrated while we waited for the lock.
            if user_version(&conn)? == from {
                let to = from + 1;
                conn.execute_batch(&format!("{step} PRAGMA user_version = {to}; COMMIT;"))
                    .context("upgrading cache schema")?;
            } else {
                conn.execute_batch("COMMIT;")?;
            }
            version = user_version(&conn)?;
        }
        if version != SCHEMA_VERSION {
            anyhow::bail!(
                "cache schema v{version} is not supported; delete the cache file to reset it"
            );
        }
        Ok(Self { conn, drafts: None })
    }

    /// Refresh SQLite's planner statistics where they are missing or
    /// stale (usually nothing to do). The app runs it as it closes.
    pub fn optimize(&self) {
        self.conn.execute_batch("PRAGMA optimize;").ok();
    }

    /// Copy the write-ahead log into the cache file as far as no reader
    /// still needs it, without waiting for anyone (after a sync pass, so
    /// the log stays short and reads stay fast).
    pub fn checkpoint(&self) {
        self.conn
            .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()))
            .ok();
    }

    /// Start a transaction that takes the write lock up front.
    fn write(&self) -> rusqlite::Result<Transaction<'_>> {
        Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
    }

    // ---- accounts ------------------------------------------------------

    pub fn accounts(&self) -> Result<Vec<Account>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, email, name, imap_host, imap_port, imap_tls, smtp_host, smtp_port,
                    smtp_tls, save_sent, signature, enabled, imap_user, smtp_user,
                    imap_pin, smtp_pin
             FROM accounts ORDER BY position, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Account {
                id: r.get(0)?,
                email: r.get(1)?,
                name: r.get(2)?,
                imap: Server {
                    host: r.get(3)?,
                    port: r.get(4)?,
                    security: Security::from_i64(r.get(5)?),
                },
                smtp: Server {
                    host: r.get(6)?,
                    port: r.get(7)?,
                    security: Security::from_i64(r.get(8)?),
                },
                save_sent: r.get(9)?,
                signature: r.get(10)?,
                enabled: r.get(11)?,
                imap_user: r.get(12)?,
                smtp_user: r.get(13)?,
                imap_pin: r.get(14)?,
                smtp_pin: r.get(15)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Save an edited account: everything but its address (the account's
    /// identity) and position. Its folders, messages and drafts stay.
    pub fn update_account(&self, a: &Account) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE accounts SET name = ?2, imap_host = ?3, imap_port = ?4, imap_tls = ?5,
                smtp_host = ?6, smtp_port = ?7, smtp_tls = ?8, save_sent = ?9, signature = ?10,
                imap_user = ?11, smtp_user = ?12, imap_pin = ?13, smtp_pin = ?14
             WHERE id = ?1",
            params![
                a.id,
                a.name,
                a.imap.host,
                a.imap.port,
                a.imap.security.as_i64(),
                a.smtp.host,
                a.smtp.port,
                a.smtp.security.as_i64(),
                a.save_sent,
                a.signature,
                a.imap_user,
                a.smtp_user,
                a.imap_pin,
                a.smtp_pin,
            ],
        )?;
        Ok(n == 1)
    }

    pub fn add_account(&self, a: &NewAccount) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO accounts (email, name, imap_host, imap_port, imap_tls,
                    smtp_host, smtp_port, smtp_tls, save_sent, imap_user, smtp_user, position)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                    (SELECT COALESCE(MAX(position), 0) + 1 FROM accounts))",
                params![
                    a.email,
                    a.name,
                    a.imap.host,
                    a.imap.port,
                    a.imap.security.as_i64(),
                    a.smtp.host,
                    a.smtp.port,
                    a.smtp.security.as_i64(),
                    a.save_sent,
                    a.imap_user,
                    a.smtp_user,
                ],
            )
            .context("saving account (is it already added?)")?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Remove an account with everything cached for it, its drafts and
    /// their attachment copies included.
    pub fn remove_account(&self, id: i64) -> Result<()> {
        let drafts: Vec<i64> = self
            .conn
            .prepare_cached("SELECT id FROM drafts WHERE account_id = ?1")?
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        self.conn
            .execute("DELETE FROM accounts WHERE id = ?1", [id])?;
        for draft in drafts {
            self.remove_draft_folder(draft);
        }
        Ok(())
    }

    /// Switch an account on or off (its cache stays either way).
    pub fn set_account_enabled(&self, id: i64, enabled: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE accounts SET enabled = ?2 WHERE id = ?1",
            params![id, enabled],
        )?;
        Ok(())
    }

    /// Move account `id` one place up (`up`) or down in the account order.
    /// The accounts are numbered 1..n in their current order first, so
    /// equal positions (older caches) still swap. False at either end.
    pub fn move_account(&self, id: i64, up: bool) -> Result<bool> {
        let tx = self.write()?;
        let mut ids: Vec<i64> = tx
            .prepare_cached("SELECT id FROM accounts ORDER BY position, id")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let Some(i) = ids.iter().position(|&a| a == id) else {
            return Ok(false);
        };
        let j = if up { i.checked_sub(1) } else { Some(i + 1) };
        let Some(j) = j.filter(|&j| j < ids.len()) else {
            return Ok(false);
        };
        ids.swap(i, j);
        for (pos, a) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE accounts SET position = ?2 WHERE id = ?1",
                params![a, pos as i64 + 1],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// Forget everything downloaded for an account, in one transaction:
    /// its messages and their bodies, and the address index built from
    /// them (bodies go with their messages; a draft replying to one keeps
    /// its threading headers but no longer marks that message). Its folders
    /// stay, with their sync fingerprints cleared (the next full sync
    /// downloads the headers again), so the drafts' copies on the server
    /// stay tracked. The account, its drafts, Outbox, invitation
    /// answers and the settings are kept. Nothing on the server changes.
    pub fn redownload(&self, account_id: i64) -> Result<()> {
        let tx = self.write()?;
        tx.execute("DELETE FROM messages WHERE account_id = ?1", [account_id])?;
        tx.execute("DELETE FROM addresses WHERE account_id = ?1", [account_id])?;
        tx.execute(
            "UPDATE folders SET sync_uidnext = NULL, sync_messages = NULL, sync_hms = NULL
             WHERE account_id = ?1",
            [account_id],
        )?;
        // Mail stored from now on is indexed as it arrives.
        tx.execute(
            "UPDATE accounts SET addr_backfill = NULL WHERE id = ?1",
            [account_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    // ---- settings ------------------------------------------------------

    /// Every saved setting, read once at start; missing keys take their
    /// defaults.
    pub fn settings(&self) -> Result<Settings> {
        let mut stmt = self.conn.prepare("SELECT key, value FROM settings")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut s = Settings::default();
        for row in rows {
            let (key, value) = row?;
            s.apply(&key, &value);
        }
        Ok(s)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // ---- folders -------------------------------------------------------

    pub fn folders(&self) -> Result<Vec<Folder>> {
        let mut stmt = self
            .conn
            .prepare_cached(&format!("SELECT {FOLDER_COLUMNS} FROM folders"))?;
        let mut folders: Vec<Folder> = stmt
            .query_map([], Folder::from_row)?
            .collect::<rusqlite::Result<_>>()?;
        folders.sort_by(|a, b| {
            (a.account_id, a.role.rank(), a.name.to_lowercase()).cmp(&(
                b.account_id,
                b.role.rank(),
                b.name.to_lowercase(),
            ))
        });
        Ok(folders)
    }

    pub fn folder(&self, id: i64) -> Result<Option<Folder>> {
        Ok(self
            .conn
            .prepare_cached(&format!(
                "SELECT {FOLDER_COLUMNS} FROM folders WHERE id = ?1"
            ))?
            .query_row([id], Folder::from_row)
            .optional()?)
    }

    pub fn folder_by_role(&self, account_id: i64, role: Role) -> Result<Option<Folder>> {
        Ok(self
            .conn
            .prepare_cached(&format!(
                "SELECT {FOLDER_COLUMNS} FROM folders WHERE account_id = ?1 AND {ROLE_SQL} = ?2
                 ORDER BY (role_override <> '') DESC, id LIMIT 1"
            ))?
            .query_row(params![account_id, role.key()], Folder::from_row)
            .optional()?)
    }

    /// The folder the server's own marks and names make `role`, whatever
    /// the user chose.
    pub fn detected_folder(&self, account_id: i64, role: Role) -> Result<Option<Folder>> {
        Ok(self
            .conn
            .prepare_cached(&format!(
                "SELECT {FOLDER_COLUMNS} FROM folders WHERE account_id = ?1 AND role = ?2
                 ORDER BY id LIMIT 1"
            ))?
            .query_row(params![account_id, role.key()], |r| {
                let mut f = Folder::from_row(r)?;
                f.role = role;
                Ok(f)
            })
            .optional()?)
    }

    /// The folder the user chose for `role` in an account, if they did.
    pub fn chosen_folder(&self, account_id: i64, role: Role) -> Result<Option<Folder>> {
        Ok(self
            .conn
            .prepare_cached(&format!(
                "SELECT {FOLDER_COLUMNS} FROM folders
                 WHERE account_id = ?1 AND role_override = ?2 ORDER BY id LIMIT 1"
            ))?
            .query_row(params![account_id, role.key()], Folder::from_row)
            .optional()?)
    }

    /// Make folder `folder_id` the account's `role` (Sent, Drafts, Trash,
    /// Archive or Junk) whatever the server marked; any folder chosen for it
    /// before stops being. `None` gives the role back to what the server
    /// marks. Nothing changes on the server, and nothing is created.
    pub fn set_role_override(
        &self,
        account_id: i64,
        role: Role,
        folder_id: Option<i64>,
    ) -> Result<()> {
        if matches!(role, Role::Inbox | Role::Other) {
            anyhow::bail!("the inbox and ordinary folders can't be chosen");
        }
        let tx = self.write()?;
        tx.execute(
            "UPDATE folders SET role_override = '' WHERE account_id = ?1 AND role_override = ?2",
            params![account_id, role.key()],
        )?;
        if let Some(id) = folder_id {
            let inbox: bool = tx.query_row(
                &format!(
                    "SELECT {ROLE_SQL} = 'inbox' FROM folders WHERE id = ?1 AND account_id = ?2"
                ),
                params![id, account_id],
                |r| r.get(0),
            )?;
            if inbox {
                anyhow::bail!("the inbox can't be used as another folder");
            }
            tx.execute(
                "UPDATE folders SET role_override = ?1 WHERE id = ?2 AND account_id = ?3",
                params![role.key(), id, account_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Insert or update a folder by its server name and return its id.
    pub fn upsert_folder(
        &self,
        account_id: i64,
        name: &str,
        display: &str,
        depth: i64,
        role: Role,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO folders (account_id, name, display, depth, role)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(account_id, name) DO UPDATE SET
                display = excluded.display, depth = excluded.depth, role = excluded.role",
            params![account_id, name, display, depth, role.key()],
        )?;
        Ok(self.conn.query_row(
            "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
            params![account_id, name],
            |r| r.get(0),
        )?)
    }

    /// Drop folders that no longer exist on the server. Their fingerprints
    /// go with them; a renamed folder comes back as a new row without one.
    pub fn retain_folders(&self, account_id: i64, keep: &[i64]) -> Result<()> {
        let keep: HashSet<i64> = keep.iter().copied().collect();
        let existing: Vec<i64> = self
            .folders()?
            .into_iter()
            .filter(|f| f.account_id == account_id && !keep.contains(&f.id))
            .map(|f| f.id)
            .collect();
        let tx = self.write()?;
        for id in existing {
            tx.execute("DELETE FROM folders WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Record a folder's UIDVALIDITY. If it changed, every cached UID is
    /// meaningless, so the folder's messages and fingerprint are dropped in
    /// one transaction. Returns true when the cache was reset.
    pub fn set_uidvalidity(&self, folder_id: i64, uidvalidity: u32) -> Result<bool> {
        let tx = self.write()?;
        let old: u32 = tx.query_row(
            "SELECT uidvalidity FROM folders WHERE id = ?1",
            [folder_id],
            |r| r.get(0),
        )?;
        if old == uidvalidity {
            return Ok(false);
        }
        tx.execute("DELETE FROM messages WHERE folder_id = ?1", [folder_id])?;
        tx.execute(
            "UPDATE folders SET uidvalidity = ?2,
                sync_uidnext = NULL, sync_messages = NULL, sync_hms = NULL
             WHERE id = ?1",
            params![folder_id, uidvalidity],
        )?;
        tx.commit()?;
        Ok(old != 0)
    }

    /// The fingerprint of the folder's last complete sync, if any.
    pub fn fingerprint(&self, folder_id: i64) -> Result<Option<Fp>> {
        let row: Option<(Option<u32>, Option<u32>, Option<i64>)> = self
            .conn
            .prepare_cached(
                "SELECT sync_uidnext, sync_messages, sync_hms FROM folders WHERE id = ?1",
            )?
            .query_row([folder_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        Ok(match row {
            Some((Some(uidnext), Some(messages), Some(hms))) => {
                u64::try_from(hms).ok().map(|hms| Fp {
                    uidnext,
                    messages,
                    hms,
                })
            }
            _ => None,
        })
    }

    /// Record the fingerprint after a complete, error-free folder sync.
    pub fn finish_folder(&self, folder_id: i64, fp: Fp) -> Result<()> {
        let hms = i64::try_from(fp.hms).context("HIGHESTMODSEQ out of range")?;
        let tx = self.write()?;
        tx.execute(
            "UPDATE folders SET sync_uidnext = ?2, sync_messages = ?3, sync_hms = ?4
             WHERE id = ?1",
            params![folder_id, fp.uidnext, fp.messages, hms],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Forget the fingerprint, so the next pass syncs the folder in full.
    pub fn clear_fingerprint(&self, folder_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE folders SET sync_uidnext = NULL, sync_messages = NULL, sync_hms = NULL
             WHERE id = ?1",
            [folder_id],
        )?;
        Ok(())
    }

    /// How many messages of the folder are cached.
    pub fn count_messages(&self, folder_id: i64) -> Result<u32> {
        Ok(self
            .conn
            .prepare_cached("SELECT COUNT(*) FROM messages WHERE folder_id = ?1")?
            .query_row([folder_id], |r| r.get(0))?)
    }

    /// The messages of these folders that are the server copies of local
    /// drafts (a draft's `server_folder`, `server_uid`, and a
    /// `server_uidvalidity` still the folder's: after a UIDVALIDITY change
    /// the uid may name another message). The Drafts entry lists the draft
    /// itself, so its copy is left out of the server's Drafts folder and a
    /// draft never shows twice.
    pub fn draft_copies(&self, folder_ids: &[i64]) -> Result<HashSet<i64>> {
        if folder_ids.is_empty() {
            return Ok(HashSet::new());
        }
        let ids = folder_ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT m.id FROM drafts d
             JOIN folders f ON f.id = d.server_folder
             JOIN messages m ON m.folder_id = d.server_folder AND m.uid = d.server_uid
             WHERE d.server_folder IN ({ids}) AND d.server_uid IS NOT NULL
               AND d.server_uidvalidity = f.uidvalidity"
        ))?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn unread_counts(&self) -> Result<HashMap<i64, i64>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT folder_id, COUNT(*) FROM messages WHERE seen = 0 GROUP BY folder_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---- message list --------------------------------------------------

    /// Ids of the messages in the given folders, newest first. `search`
    /// filters on subject, sender, recipients and preview (a substring,
    /// ignoring case and accents: `crate::fold`). With the list index, one
    /// folder is read in order without sorting.
    pub fn message_ids(&self, folder_ids: &[i64], search: &str) -> Result<Vec<i64>> {
        self.message_ids_filtered(folder_ids, search, Filter::All)
    }

    /// [`Db::message_ids`] narrowed by `filter`. Unread reads the
    /// (folder, seen) index and sorts only the unread rows.
    pub fn message_ids_filtered(
        &self,
        folder_ids: &[i64],
        search: &str,
        filter: Filter,
    ) -> Result<Vec<i64>> {
        if folder_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self
            .conn
            .prepare_cached(&list_query(folder_ids, search, filter))?;
        let search = search.trim();
        let rows = if search.is_empty() {
            stmt.query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        } else {
            stmt.query_map([search_needle(search)], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        Ok(rows)
    }

    /// One list row (read on demand as rows scroll into view).
    pub fn header(&self, id: i64) -> Result<Option<Header>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT subject, from_name, from_addr, date, seen, flagged, answered,
                        has_attach, preview, to_addrs,
                        EXISTS(SELECT 1 FROM drafts
                               WHERE reply_to_msg = messages.id AND state = 0),
                        folder_id, account_id
                 FROM messages WHERE id = ?1",
            )?
            .query_row([id], |r| {
                Ok(Header {
                    subject: r.get(0)?,
                    from_name: r.get(1)?,
                    from_addr: r.get(2)?,
                    date: r.get(3)?,
                    seen: r.get(4)?,
                    flagged: r.get(5)?,
                    answered: r.get(6)?,
                    has_attach: r.get(7)?,
                    preview: r.get(8)?,
                    to: r.get(9)?,
                    has_draft: r.get(10)?,
                    folder_id: r.get(11)?,
                    account_id: r.get(12)?,
                })
            })
            .optional()?)
    }

    // ---- sync ----------------------------------------------------------

    pub fn known_uids(&self, folder_id: i64) -> Result<HashSet<u32>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT uid FROM messages WHERE folder_id = ?1")?;
        let rows = stmt.query_map([folder_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn delete_uids(&mut self, folder_id: i64, uids: &[u32]) -> Result<()> {
        let tx = self.write()?;
        {
            let mut stmt =
                tx.prepare_cached("DELETE FROM messages WHERE folder_id = ?1 AND uid = ?2")?;
            for uid in uids {
                stmt.execute(params![folder_id, uid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn insert_headers(
        &mut self,
        account_id: i64,
        folder_id: i64,
        headers: &[NewHeader],
    ) -> Result<()> {
        let tx = self.write()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO messages (account_id, folder_id, uid, msgid, subject,
                    from_name, from_addr, to_addrs, cc_addrs, date, size, seen, flagged,
                    answered, has_attach)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            )?;
            let (role, me) = index_context(&tx, folder_id)?;
            let mut index = tx.prepare_cached(UPSERT_ADDRESS)?;
            for h in headers {
                let added = stmt.execute(params![
                    account_id,
                    folder_id,
                    h.uid,
                    h.msgid,
                    h.subject,
                    h.from_name,
                    h.from_addr,
                    h.to,
                    h.cc,
                    h.date,
                    h.size,
                    h.seen,
                    h.flagged,
                    h.answered,
                    h.has_attach,
                ])?;
                // A row already cached was indexed when it was stored.
                if added == 1 {
                    index_header(&mut index, account_id, &me, role, h)?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Take up to `chunk` of the messages cached before the address index
    /// existed into it, newest first. The mark moves in the same
    /// transaction, so an interrupted run never counts a message twice.
    /// True while some are left.
    pub fn backfill_addresses(&mut self, account_id: i64, chunk: usize) -> Result<bool> {
        let chunk = chunk.max(1);
        let tx = self.write()?;
        let more = {
            let found = tx
                .query_row(
                    "SELECT addr_backfill, email FROM accounts WHERE id = ?1",
                    [account_id],
                    |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, String>(1)?)),
                )
                .optional()?;
            let Some((Some(mark), email)) = found else {
                return Ok(false);
            };
            let me = email.to_lowercase();
            let mut stmt = tx.prepare_cached(
                "SELECT m.id, CASE WHEN f.role_override <> '' THEN f.role_override ELSE f.role END, m.from_name, m.from_addr, m.to_addrs, m.cc_addrs, m.date
                 FROM messages m JOIN folders f ON f.id = m.folder_id
                 WHERE m.account_id = ?1 AND m.id <= ?2
                 ORDER BY m.id DESC LIMIT ?3",
            )?;
            let batch = stmt
                .query_map(params![account_id, mark, chunk as i64], |r| {
                    let h = NewHeader {
                        from_name: r.get(2)?,
                        from_addr: r.get(3)?,
                        to: r.get(4)?,
                        cc: r.get(5)?,
                        date: r.get(6)?,
                        ..NewHeader::default()
                    };
                    Ok((
                        r.get::<_, i64>(0)?,
                        Role::from_key(&r.get::<_, String>(1)?),
                        h,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut index = tx.prepare_cached(UPSERT_ADDRESS)?;
            for (_, role, h) in &batch {
                index_header(&mut index, account_id, &me, *role, h)?;
            }
            let next = match batch.last() {
                Some((id, _, _)) if batch.len() == chunk => Some(id - 1),
                _ => None,
            };
            tx.execute(
                "UPDATE accounts SET addr_backfill = ?2 WHERE id = ?1",
                params![account_id, next],
            )?;
            next.is_some()
        };
        tx.commit()?;
        Ok(more)
    }

    /// Drop the address index of `account_id` and mark all its cached mail
    /// for the backfill, which builds the index again from the messages.
    pub fn rebuild_addresses(&mut self, account_id: i64) -> Result<()> {
        let tx = self.write()?;
        tx.execute("DELETE FROM addresses WHERE account_id = ?1", [account_id])?;
        tx.execute(
            "UPDATE accounts SET addr_backfill =
                (SELECT MAX(id) FROM messages WHERE account_id = ?1)
             WHERE id = ?1",
            [account_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Up to `limit` indexed addresses of `account_id` whose address or
    /// display name starts with `prefix` (matched lower-cased): the most
    /// used first, then the most recent. Two index range searches, so it
    /// stays well under a millisecond on 10,000 addresses.
    pub fn suggest_addresses(
        &self,
        account_id: i64,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<Recipient>> {
        let low = fold(prefix.trim());
        if low.is_empty() {
            return Ok(Vec::new());
        }
        // Every text starting with `low` sorts below this.
        let high = format!("{low}{}", char::MAX);
        let mut stmt = self.conn.prepare_cached(
            "SELECT name, addr FROM addresses
             WHERE account_id = ?1
               AND ((addr >= ?2 AND addr < ?3) OR (folded >= ?2 AND folded < ?3))
             ORDER BY weight DESC, last_seen DESC, addr LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![account_id, low, high, limit as i64], |r| {
            Ok(Recipient {
                name: r.get(0)?,
                addr: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Bring cached flags in line with the server's (UID → flag bits),
    /// leaving `skip` (UIDs with a local change in flight) untouched.
    /// Returns how many rows changed.
    pub fn apply_flags(
        &mut self,
        folder_id: i64,
        remote: &HashMap<u32, u8>,
        skip: &HashSet<u32>,
    ) -> Result<usize> {
        let tx = self.write()?;
        let mut changed = 0;
        {
            let mut select = tx.prepare_cached(
                "SELECT id, uid, seen, flagged, answered FROM messages WHERE folder_id = ?1",
            )?;
            let rows: Vec<(i64, u32, bool, bool, bool)> = select
                .query_map([folder_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })?
                .collect::<rusqlite::Result<_>>()?;
            let mut update = tx.prepare_cached(
                "UPDATE messages SET seen = ?2, flagged = ?3, answered = ?4 WHERE id = ?1",
            )?;
            for (id, uid, seen, fl, ans) in rows {
                let Some(&bits) = remote.get(&uid) else {
                    continue;
                };
                if skip.contains(&uid) {
                    continue;
                }
                let (s, f, a) = (bits & SEEN != 0, bits & FLAGGED != 0, bits & ANSWERED != 0);
                if (s, f, a) != (seen, fl, ans) {
                    update.execute(params![id, s, f, a])?;
                    changed += 1;
                }
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Apply the flags of only the UIDs in `delta` (a CHANGEDSINCE reply),
    /// leaving `skip` untouched; UIDs not cached are ignored. Returns how
    /// many rows changed.
    pub fn apply_flag_delta(
        &mut self,
        folder_id: i64,
        delta: &HashMap<u32, u8>,
        skip: &HashSet<u32>,
    ) -> Result<usize> {
        let tx = self.write()?;
        let mut changed = 0;
        {
            let mut update = tx.prepare_cached(
                "UPDATE messages SET seen = ?3, flagged = ?4, answered = ?5
                 WHERE folder_id = ?1 AND uid = ?2
                   AND (seen != ?3 OR flagged != ?4 OR answered != ?5)",
            )?;
            for (&uid, &bits) in delta {
                if skip.contains(&uid) {
                    continue;
                }
                let (s, f, a) = (bits & SEEN != 0, bits & FLAGGED != 0, bits & ANSWERED != 0);
                changed += update.execute(params![folder_id, uid, s, f, a])?;
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Messages among a folder's newest `window` whose body is not cached
    /// yet and is at most `max_size`, newest first, as (id, UID, size).
    pub fn missing_bodies(
        &self,
        folder_id: i64,
        window: usize,
        max_size: i64,
    ) -> Result<Vec<(i64, u32, i64)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, uid, size FROM (
                 SELECT id, uid, size, date, has_body FROM messages
                 WHERE folder_id = ?1 ORDER BY date DESC, id DESC LIMIT ?3)
             WHERE has_body = 0 AND size <= ?2
             ORDER BY date DESC, id DESC",
        )?;
        let rows = stmt.query_map(params![folder_id, max_size, window as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---- single messages ----------------------------------------------

    pub fn message(&self, id: i64) -> Result<Option<Message>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, account_id, folder_id, uid, msgid, subject, from_name, from_addr,
                        to_addrs, cc_addrs, date, seen, flagged,
                        (SELECT raw FROM bodies WHERE message_id = messages.id)
                 FROM messages WHERE id = ?1",
                [id],
                |r| {
                    Ok(Message {
                        id: r.get(0)?,
                        account_id: r.get(1)?,
                        folder_id: r.get(2)?,
                        uid: r.get(3)?,
                        msgid: r.get(4)?,
                        subject: r.get(5)?,
                        from_name: r.get(6)?,
                        from_addr: r.get(7)?,
                        to: r.get(8)?,
                        cc: r.get(9)?,
                        date: r.get(10)?,
                        seen: r.get(11)?,
                        flagged: r.get(12)?,
                        raw: r.get(13)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn set_body(&mut self, id: i64, raw: &[u8], preview: &str, has_attach: bool) -> Result<()> {
        let tx = self.write()?;
        tx.execute(
            "INSERT OR REPLACE INTO bodies (message_id, raw) VALUES (?1, ?2)",
            params![id, raw],
        )?;
        tx.execute(
            "UPDATE messages SET preview = ?2, has_attach = ?3, has_body = 1 WHERE id = ?1",
            params![id, preview, has_attach],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Store several fetched bodies (`id`, raw, preview, has attachment) in
    /// one transaction: one write-lock acquisition and one commit for the
    /// batch (docs/PERF.md 3.2, 5.1). All or nothing, so a failure leaves
    /// every message still `has_body = 0` and the next prefetch pass retries.
    pub fn set_bodies(&mut self, items: &[(i64, &[u8], &str, bool)]) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let tx = self.write()?;
        {
            let mut put = tx.prepare_cached(
                "INSERT OR REPLACE INTO bodies (message_id, raw) VALUES (?1, ?2)",
            )?;
            let mut mark = tx.prepare_cached(
                "UPDATE messages SET preview = ?2, has_attach = ?3, has_body = 1 WHERE id = ?1",
            )?;
            for (id, raw, preview, has_attach) in items {
                put.execute(params![id, raw])?;
                mark.execute(params![id, preview, has_attach])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn set_seen(&self, id: i64, seen: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE messages SET seen = ?2 WHERE id = ?1",
            params![id, seen],
        )?;
        Ok(())
    }

    pub fn set_flagged(&self, id: i64, flagged: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE messages SET flagged = ?2 WHERE id = ?1",
            params![id, flagged],
        )?;
        Ok(())
    }

    pub fn set_answered(&self, id: i64) -> Result<()> {
        self.conn
            .execute("UPDATE messages SET answered = 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn delete_message(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM messages WHERE id = ?1", [id])?;
        Ok(())
    }

    /// The cached messages among `ids`, in no particular order, read a
    /// chunk of ids per query (a selection can be a whole folder).
    pub fn briefs(&self, ids: &[i64]) -> Result<Vec<Brief>> {
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(500) {
            let marks = vec!["?"; chunk.len()].join(",");
            let mut stmt = self.conn.prepare(&format!(
                "SELECT id, account_id, folder_id, uid, seen, flagged
                 FROM messages WHERE id IN ({marks})"
            ))?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk), |r| {
                Ok(Brief {
                    id: r.get(0)?,
                    account_id: r.get(1)?,
                    folder_id: r.get(2)?,
                    uid: r.get(3)?,
                    seen: r.get(4)?,
                    flagged: r.get(5)?,
                })
            })?;
            for b in rows {
                out.push(b?);
            }
        }
        Ok(out)
    }

    /// [`Db::set_seen`] for many messages, in one transaction.
    pub fn set_seen_many(&self, ids: &[i64], seen: bool) -> Result<()> {
        self.each(ids, "UPDATE messages SET seen = ?2 WHERE id = ?1", seen)
    }

    /// Mark every cached message of the folder read. Returns the UIDs that
    /// were unread and the highest UID cached (0: none), the end of the
    /// range the server is told about.
    pub fn mark_folder_read(&self, folder_id: i64) -> Result<(Vec<u32>, u32)> {
        let tx = self.write()?;
        let uids = tx
            .prepare(
                "UPDATE messages SET seen = 1 WHERE folder_id = ?1 AND seen = 0 RETURNING uid",
            )?
            .query_map([folder_id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<u32>>>()?;
        let highest = tx.query_row(
            "SELECT COALESCE(MAX(uid), 0) FROM messages WHERE folder_id = ?1",
            [folder_id],
            |r| r.get(0),
        )?;
        tx.commit()?;
        Ok((uids, highest))
    }

    /// [`Db::set_flagged`] for many messages, in one transaction.
    pub fn set_flagged_many(&self, ids: &[i64], flagged: bool) -> Result<()> {
        self.each(
            ids,
            "UPDATE messages SET flagged = ?2 WHERE id = ?1",
            flagged,
        )
    }

    /// [`Db::delete_message`] for many messages, in one transaction.
    pub fn delete_messages(&self, ids: &[i64]) -> Result<()> {
        let tx = self.write()?;
        {
            let mut stmt = tx.prepare("DELETE FROM messages WHERE id = ?1")?;
            for id in ids {
                stmt.execute([id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn each(&self, ids: &[i64], sql: &str, on: bool) -> Result<()> {
        let tx = self.write()?;
        {
            let mut stmt = tx.prepare(sql)?;
            for id in ids {
                stmt.execute(params![id, on])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    // ---- drafts --------------------------------------------------------

    /// Save a new draft and return its id. A folder left over from an
    /// earlier draft with the same id is cleared first.
    pub fn insert_draft(&self, d: &DraftRow) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO drafts (account_id, kind, reply_to_msg, in_reply_to, refs, to_addrs,
                cc_addrs, bcc_addrs, subject, body, message_id, created, updated, quoted_html)
             VALUES (?1, ?2, (SELECT id FROM messages WHERE id = ?3), ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12, ?13)",
            params![
                d.account_id,
                d.kind,
                d.reply_to_msg,
                d.in_reply_to,
                d.refs,
                d.to,
                d.cc,
                d.bcc,
                d.subject,
                d.body,
                d.message_id,
                chrono::Utc::now().timestamp(),
                d.quoted_html,
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.remove_draft_folder(id);
        Ok(id)
    }

    /// Save the fields of draft `d.id` with one UPDATE (attachments are
    /// filed on their own). False if no such draft is saved any more, or
    /// it is in the outbox (whose rows never change).
    pub fn update_draft(&self, d: &DraftRow) -> Result<bool> {
        let changed = self
            .conn
            .prepare_cached(
                "UPDATE drafts SET account_id = ?2, kind = ?3,
                    reply_to_msg = (SELECT id FROM messages WHERE id = ?4),
                    in_reply_to = ?5, refs = ?6, to_addrs = ?7, cc_addrs = ?8, bcc_addrs = ?9,
                    subject = ?10, body = ?11, message_id = ?12, updated = ?13,
                    quoted_html = ?14, rev = rev + 1
                 WHERE id = ?1 AND state = 0",
            )?
            .execute(params![
                d.id,
                d.account_id,
                d.kind,
                d.reply_to_msg,
                d.in_reply_to,
                d.refs,
                d.to,
                d.cc,
                d.bcc,
                d.subject,
                d.body,
                d.message_id,
                chrono::Utc::now().timestamp(),
                d.quoted_html,
            ])?;
        Ok(changed > 0)
    }

    /// Whether `id` is a saved draft (`state` 0), not a message in the
    /// outbox: those are never opened for editing from a stale list.
    pub fn is_saved_draft(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn
            .prepare_cached("SELECT EXISTS(SELECT 1 FROM drafts WHERE id = ?1 AND state = 0)")?
            .query_row([id], |r| r.get(0))?)
    }

    pub fn draft(&self, id: i64) -> Result<Option<DraftRow>> {
        Ok(self
            .conn
            .prepare_cached(&format!("SELECT {DRAFT_COLUMNS} FROM drafts WHERE id = ?1"))?
            .query_row([id], DraftRow::from_row)
            .optional()?)
    }

    /// Ids of the saved drafts, last saved first. `search` filters on the
    /// subject, recipients and body (a substring, ignoring case and
    /// accents, like [`Db::message_ids`]).
    pub fn draft_ids(&self, search: &str) -> Result<Vec<i64>> {
        let search = search.trim();
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT id FROM drafts WHERE state = 0 AND {DRAFT_MATCH}
             ORDER BY updated DESC, id DESC"
        ))?;
        let rows = stmt.query_map(params![search, search_needle(search)], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// One row of the Drafts view.
    pub fn draft_header(&self, id: i64) -> Result<Option<DraftHeader>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT to_addrs, subject, substr(body, 1, 400), updated,
                        EXISTS(SELECT 1 FROM draft_attachments WHERE draft_id = drafts.id)
                 FROM drafts WHERE id = ?1",
            )?
            .query_row([id], |r| {
                Ok(DraftHeader {
                    to: r.get(0)?,
                    subject: r.get(1)?,
                    preview: r.get(2)?,
                    updated: r.get(3)?,
                    has_attach: r.get(4)?,
                })
            })
            .optional()?)
    }

    /// How many drafts are saved.
    pub fn count_drafts(&self) -> Result<i64> {
        Ok(self
            .conn
            .prepare_cached("SELECT COUNT(*) FROM drafts WHERE state = 0")?
            .query_row([], |r| r.get(0))?)
    }

    /// The last saved draft replying to message `message_id`.
    pub fn draft_for_message(&self, message_id: i64) -> Result<Option<i64>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT id FROM drafts WHERE reply_to_msg = ?1 AND state = 0
                 ORDER BY updated DESC, id DESC LIMIT 1",
            )?
            .query_row([message_id], |r| r.get(0))
            .optional()?)
    }

    /// Delete a draft, its attachment rows and their copies. Returns its
    /// copy on the server, which the caller removes there.
    pub fn delete_draft(&self, id: i64) -> Result<Option<ServerCopy>> {
        let tx = self.write()?;
        let copy = server_copy(&tx, id)?;
        tx.execute("DELETE FROM drafts WHERE id = ?1", [id])?;
        tx.commit()?;
        self.remove_draft_folder(id);
        Ok(copy)
    }

    /// Account `account_id`'s drafts changed since their last copy went to
    /// the server (or never copied), oldest first, as (id, change count).
    pub fn drafts_to_upload(&self, account_id: i64) -> Result<Vec<(i64, i64)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, rev FROM drafts WHERE account_id = ?1 AND state = 0 AND rev > server_rev
             ORDER BY id",
        )?;
        let rows = stmt.query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Record `copy` (`None`: saved, but its UID is unknown) as the server
    /// copy of draft `id` at change `rev`, if it is still a draft.
    pub fn record_server_copy(
        &self,
        id: i64,
        rev: i64,
        copy: Option<&ServerCopy>,
    ) -> Result<Recorded> {
        let tx = self.write()?;
        let old = server_copy(&tx, id)?;
        let n = tx.execute(
            "UPDATE drafts SET server_rev = max(server_rev, ?2), server_folder = ?3,
                server_uidvalidity = ?4, server_uid = ?5
             WHERE id = ?1 AND state = 0",
            params![
                id,
                rev,
                copy.map(|c| c.folder_id),
                copy.map(|c| c.uidvalidity),
                copy.map(|c| c.uid),
            ],
        )?;
        tx.commit()?;
        Ok(if n == 0 {
            Recorded::NotADraft
        } else {
            Recorded::Replaced(old)
        })
    }

    /// Copy an attachment into draft `draft_id`'s folder and record it.
    pub fn add_draft_file(
        &self,
        draft_id: i64,
        name: &str,
        mime: &str,
        data: &[u8],
    ) -> Result<DraftFile> {
        let dir = self.draft_folder(draft_id)?;
        let tx = self.write()?;
        tx.execute(
            "INSERT INTO draft_attachments (draft_id, name, mime, size, file)
             VALUES (?1, ?2, ?3, ?4, '')",
            params![draft_id, name, mime, data.len() as i64],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "UPDATE draft_attachments SET file = ?2 WHERE id = ?1",
            params![id, id.to_string()],
        )?;
        tx.execute("UPDATE drafts SET rev = rev + 1 WHERE id = ?1", [draft_id])?;
        let path = dir.join(id.to_string());
        let written = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, data));
        if let Err(e) = written {
            std::fs::remove_file(&path).ok();
            return Err(e).with_context(|| format!("saving a copy of {name}"));
        }
        tx.commit()?;
        Ok(DraftFile {
            id,
            name: name.into(),
            mime: mime.into(),
            size: data.len() as i64,
        })
    }

    /// Drop one attachment of a draft and its copy.
    pub fn remove_draft_file(&self, draft_id: i64, file_id: i64) -> Result<()> {
        let n = self.conn.execute(
            "DELETE FROM draft_attachments WHERE id = ?1 AND draft_id = ?2",
            params![file_id, draft_id],
        )?;
        if n > 0 {
            self.conn
                .execute("UPDATE drafts SET rev = rev + 1 WHERE id = ?1", [draft_id])?;
        }
        if let Ok(dir) = self.draft_folder(draft_id) {
            std::fs::remove_file(dir.join(file_id.to_string())).ok();
        }
        Ok(())
    }

    /// A draft's attachments, in the order they were added.
    pub fn draft_files(&self, draft_id: i64) -> Result<Vec<DraftFile>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, name, mime, size FROM draft_attachments WHERE draft_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([draft_id], |r| {
            Ok(DraftFile {
                id: r.get(0)?,
                name: r.get(1)?,
                mime: r.get(2)?,
                size: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Read an attachment's copy back, refusing one larger than `limit`.
    /// The path comes from the ids alone, never from a stored name.
    pub fn read_draft_file(&self, draft_id: i64, file: &DraftFile, limit: u64) -> Result<Vec<u8>> {
        use std::io::Read;
        let path = self.draft_folder(draft_id)?.join(file.id.to_string());
        let mut data = Vec::new();
        std::fs::File::open(&path)
            .and_then(|f| f.take(limit.saturating_add(1)).read_to_end(&mut data))
            .with_context(|| format!("reading the saved copy of {}", file.name))?;
        if data.len() as u64 > limit {
            anyhow::bail!("{} is over the attachment limit", file.name);
        }
        Ok(data)
    }

    fn draft_folder(&self, draft_id: i64) -> Result<PathBuf> {
        let dir = self
            .drafts
            .as_ref()
            .context("this cache has no drafts folder")?;
        Ok(dir.join(draft_id.to_string()))
    }

    /// Best effort: a folder left behind is cleared when its id is reused.
    fn remove_draft_folder(&self, draft_id: i64) {
        if let Ok(dir) = self.draft_folder(draft_id) {
            std::fs::remove_dir_all(dir).ok();
        }
    }

    // ---- outbox --------------------------------------------------------
    //
    // Each step is one conditional statement on `state`, so the UI, the
    // periodic pass and a send thread can never move a row twice.

    /// Send saved draft `id`: it enters the undo window. False if it is
    /// not a saved draft.
    pub fn outbox_send(&self, id: i64) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE drafts SET state = 1, raw = NULL, attempts = 0, next_try = NULL,
                last_error = NULL, updated = ?2
             WHERE id = ?1 AND state = 0",
            params![id, chrono::Utc::now().timestamp()],
        )?;
        Ok(n > 0)
    }

    /// Store the built message of pending row `id`, unless `cancel` is set
    /// by then. The flag is read holding the write lock, so an Undo (which
    /// sets it, then clears `raw`) can never be overtaken by a late build.
    pub fn store_raw(&self, id: i64, raw: &[u8], cancel: &AtomicBool) -> Result<bool> {
        let tx = self.write()?;
        if cancel.load(Ordering::SeqCst) {
            return Ok(false);
        }
        let n = tx.execute(
            "UPDATE drafts SET raw = ?2 WHERE id = ?1 AND state = 1",
            params![id, raw],
        )?;
        tx.commit()?;
        Ok(n > 0)
    }

    /// The undo window of row `id` is over: queue it, once it is built.
    pub fn outbox_queue(&self, id: i64) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE drafts SET state = 2 WHERE id = ?1 AND state = 1 AND raw IS NOT NULL",
            [id],
        )?;
        Ok(n > 0)
    }

    /// Take outbox row `id` back as a draft (Undo, or Edit): from the undo
    /// window, from the queue before a send takes it, or after a failed or
    /// unconfirmed send. Its built message is dropped. Never an
    /// invitation's reply (it has no draft to go back to).
    pub fn outbox_unsend(&self, id: i64) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE drafts SET state = 0, raw = NULL, attempts = 0, next_try = NULL,
                last_error = NULL
             WHERE id = ?1 AND state IN (1, 2, 4, 5) AND kind != 4",
            [id],
        )?;
        Ok(n > 0)
    }

    /// Retry: queue a failed or unconfirmed row again, due now.
    pub fn outbox_retry(&self, id: i64) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE drafts SET state = 2, attempts = 0, next_try = NULL, last_error = NULL
             WHERE id = ?1 AND state IN (4, 5) AND raw IS NOT NULL",
            [id],
        )?;
        Ok(n > 0)
    }

    /// A send takes queued row `id`. One statement moves it to Sending, so
    /// two sends racing for it never both get it. `None`: not queued (any
    /// more): undone, discarded or taken.
    pub fn claim_outbox(&self, id: i64) -> Result<Option<Outbound>> {
        let n = self.conn.execute(
            "UPDATE drafts SET state = 3, attempts = attempts + 1
             WHERE id = ?1 AND state = 2 AND raw IS NOT NULL",
            [id],
        )?;
        if n == 0 {
            return Ok(None);
        }
        Ok(self
            .conn
            .query_row(
                "SELECT to_addrs, cc_addrs, bcc_addrs, raw, reply_to_msg, attempts
                 FROM drafts WHERE id = ?1",
                [id],
                |r| {
                    Ok(Outbound {
                        to: r.get(0)?,
                        cc: r.get(1)?,
                        bcc: r.get(2)?,
                        raw: r.get(3)?,
                        reply_to_msg: r.get(4)?,
                        attempts: r.get(5)?,
                    })
                },
            )
            .optional()?)
    }

    /// A send of row `id` ended without delivering it: queued again (due
    /// at `next_try`), failed or unconfirmed, with the reason.
    pub fn outbox_result(
        &self,
        id: i64,
        state: Outbox,
        next_try: Option<i64>,
        error: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE drafts SET state = ?2, next_try = ?3, last_error = ?4
             WHERE id = ?1 AND state = 3",
            params![id, state as i64, next_try, error],
        )?;
        Ok(())
    }

    /// At start: a send that was running when the app stopped may have
    /// gone out, so it becomes Unconfirmed (never resent automatically).
    pub fn recover_outbox(&self, error: &str) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE drafts SET state = 5, next_try = NULL, last_error = ?1 WHERE state = 3",
            [error],
        )?)
    }

    /// Queued rows due at `now` (unix seconds), oldest first, as (id,
    /// account id).
    pub fn outbox_due(&self, now: i64) -> Result<Vec<(i64, i64)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, account_id FROM drafts
             WHERE state = 2 AND (next_try IS NULL OR next_try <= ?1) ORDER BY id",
        )?;
        let rows = stmt.query_map([now], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Rows in the undo window, with whether their message is built.
    pub fn outbox_pending(&self) -> Result<Vec<(i64, bool)>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id, raw IS NOT NULL FROM drafts WHERE state = 1 ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Ids of the outbox rows, newest first. `search` filters like
    /// [`Db::draft_ids`].
    pub fn outbox_ids(&self, search: &str) -> Result<Vec<i64>> {
        let search = search.trim();
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT id FROM drafts WHERE state > 0 AND {DRAFT_MATCH}
             ORDER BY updated DESC, id DESC"
        ))?;
        let rows = stmt.query_map(params![search, search_needle(search)], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn outbox_entry(&self, id: i64) -> Result<Option<OutboxEntry>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT account_id, to_addrs, subject, state, next_try, last_error, updated,
                        EXISTS(SELECT 1 FROM draft_attachments WHERE draft_id = drafts.id),
                        kind = 4, body
                 FROM drafts WHERE id = ?1 AND state > 0",
            )?
            .query_row([id], |r| {
                Ok(OutboxEntry {
                    account_id: r.get(0)?,
                    to: r.get(1)?,
                    subject: r.get(2)?,
                    state: Outbox::from_code(r.get(3)?).unwrap_or(Outbox::Failed),
                    next_try: r.get(4)?,
                    last_error: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                    updated: r.get(6)?,
                    has_attach: r.get(7)?,
                    invite_reply: r.get(8)?,
                    body: r.get::<_, Option<String>>(9)?.unwrap_or_default(),
                })
            })
            .optional()?)
    }

    /// How many messages are in the outbox.
    pub fn count_outbox(&self) -> Result<i64> {
        Ok(self
            .conn
            .prepare_cached("SELECT COUNT(*) FROM drafts WHERE state > 0")?
            .query_row([], |r| r.get(0))?)
    }

    /// Delete an outbox row no send holds (queued, failed or unconfirmed)
    /// with its attachment copies. False if it is not such a row; also
    /// returns its server copy, which the caller removes there.
    pub fn discard_outbox(&self, id: i64) -> Result<(bool, Option<ServerCopy>)> {
        let tx = self.write()?;
        let copy = server_copy(&tx, id)?;
        let n = tx.execute(
            "DELETE FROM drafts WHERE id = ?1 AND state IN (2, 4, 5)",
            [id],
        )?;
        tx.commit()?;
        if n == 0 {
            return Ok((false, None));
        }
        self.remove_draft_folder(id);
        Ok((true, copy))
    }

    /// Drafts and outbox rows of account `id` (all lost with it).
    pub fn count_unsent(&self, account_id: i64) -> Result<i64> {
        Ok(self
            .conn
            .prepare_cached("SELECT COUNT(*) FROM drafts WHERE account_id = ?1")?
            .query_row([account_id], |r| r.get(0))?)
    }

    // ---- meeting invitations ---------------------------------------------

    /// Queue `raw`, the built reply to an invitation, as outbox row `d`
    /// (kind [`INVITE_REPLY`]), due now. It never passes through Drafts or
    /// the undo window, so its calendar part can't be lost to a rebuild.
    pub fn queue_reply(&self, d: &DraftRow, raw: &[u8]) -> Result<i64> {
        let now = chrono::Utc::now().timestamp();
        self.conn.execute(
            "INSERT INTO drafts (account_id, kind, to_addrs, subject, body, message_id, created,
                updated, state, raw)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, 2, ?8)",
            params![
                d.account_id,
                INVITE_REPLY,
                d.to,
                d.subject,
                d.body,
                d.message_id,
                now,
                raw
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.remove_draft_folder(id);
        Ok(id)
    }

    /// Record the answer `partstat` (`ical::Answer`) to invite `uid`
    /// (occurrence `recurrence_id`) at its `sequence`.
    pub fn record_rsvp(
        &self,
        account_id: i64,
        uid: &str,
        recurrence_id: &str,
        sequence: i64,
        partstat: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO rsvp (account_id, uid, recurrence_id, sequence, partstat,
                answered) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                account_id,
                uid,
                recurrence_id,
                sequence,
                partstat,
                chrono::Utc::now().timestamp()
            ],
        )?;
        Ok(())
    }

    /// The answer recorded for invite `uid` (occurrence `recurrence_id`):
    /// (the invite's sequence then, the answer).
    pub fn rsvp(
        &self,
        account_id: i64,
        uid: &str,
        recurrence_id: &str,
    ) -> Result<Option<(i64, i64)>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT IFNULL(sequence, 0), IFNULL(partstat, 0) FROM rsvp
                 WHERE account_id = ?1 AND uid = ?2 AND recurrence_id = ?3",
            )?
            .query_row(params![account_id, uid, recurrence_id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?)
    }

    // ---- remote images -------------------------------------------------

    /// Whether the user chose to always load remote images from `address`,
    /// a bare From address. Only that exact address matches: never a
    /// display name, a domain or a `Name <address>` form.
    pub fn remote_allowed(&self, address: &str) -> Result<bool> {
        let Some(address) = allow_key(address) else {
            return Ok(false);
        };
        Ok(self
            .conn
            .prepare_cached("SELECT 1 FROM remote_allow WHERE address = ?1")?
            .exists([address])?)
    }

    /// Always load remote images from `address` (a bare From address).
    pub fn allow_remote(&self, address: &str) -> Result<()> {
        let address = allow_key(address).context("not a bare email address")?;
        self.conn.execute(
            "INSERT OR IGNORE INTO remote_allow (address, created) VALUES (?1, ?2)",
            params![address, chrono::Utc::now().timestamp()],
        )?;
        Ok(())
    }

    /// The senders whose remote images load without asking, sorted.
    pub fn remote_allow_list(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT address FROM remote_allow ORDER BY address")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn forget_remote(&self, address: &str) -> Result<()> {
        if let Some(address) = allow_key(address) {
            self.conn
                .execute("DELETE FROM remote_allow WHERE address = ?1", [address])?;
        }
        Ok(())
    }

    pub fn forget_all_remote(&self) -> Result<()> {
        self.conn.execute("DELETE FROM remote_allow", [])?;
        Ok(())
    }
}

/// The allowlist key for a bare address: lower-cased, or `None` for
/// anything that is not a single `local@domain`.
fn allow_key(address: &str) -> Option<String> {
    let a = address.trim();
    let (local, domain) = a.split_once('@')?;
    let bad = |c: char| c.is_whitespace() || c.is_control() || "<>\",;".contains(c);
    (!local.is_empty() && !domain.is_empty() && !domain.contains('@') && !a.contains(bad))
        .then(|| a.to_lowercase())
}

/// The address index's matching form of a name or a typed prefix.
fn fold(s: &str) -> String {
    s.to_lowercase()
}

/// The role of folder `folder_id` and its account's own address
/// (lower-cased), which the address index leaves out.
fn index_context(conn: &Connection, folder_id: i64) -> rusqlite::Result<(Role, String)> {
    let found = conn
        .query_row(
            "SELECT CASE WHEN f.role_override <> '' THEN f.role_override ELSE f.role END, a.email
             FROM folders f JOIN accounts a ON a.id = f.account_id
             WHERE f.id = ?1",
            [folder_id],
            |r| {
                Ok((
                    Role::from_key(&r.get::<_, String>(0)?),
                    r.get::<_, String>(1)?.to_lowercase(),
                ))
            },
        )
        .optional()?;
    Ok(found.unwrap_or_default())
}

/// Add the addresses of one cached message to the index (`stmt` is
/// `UPSERT_ADDRESS`): its sender, and for mail in Sent its To and Cc. Junk
/// and Drafts add nothing, and neither does the account's own address `me`
/// or anything that is not a single `local@domain`.
fn index_header(
    stmt: &mut rusqlite::CachedStatement<'_>,
    account_id: i64,
    me: &str,
    role: Role,
    h: &NewHeader,
) -> rusqlite::Result<()> {
    if matches!(role, Role::Junk | Role::Drafts) {
        return Ok(());
    }
    let mut add = |name: &str, addr: &str, weight: i64| {
        let Some(addr) = allow_key(addr).filter(|a| a != me) else {
            return Ok(());
        };
        let name = name.trim();
        // A name that only repeats the address adds nothing.
        // (Nor does one with control characters: it would not fit a
        // one-line field.)
        let name = if name.eq_ignore_ascii_case(&addr) || name.contains(char::is_control) {
            ""
        } else {
            name
        };
        stmt.execute(params![account_id, addr, name, fold(name), h.date, weight])
            .map(drop)
    };
    add(&h.from_name, &h.from_addr, FROM_WEIGHT)?;
    if role == Role::Sent {
        let mut list = smtp::split_addresses(&h.to);
        list.extend(smtp::split_addresses(&h.cc));
        for entry in &list {
            let (name, addr) = smtp::mailbox_parts(entry);
            add(&name, addr, SENT_WEIGHT)?;
        }
    }
    Ok(())
}

fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(WORKER_BUSY)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;
         PRAGMA journal_size_limit = 16777216;",
    )?;
    // `fold(text)`: the search form of a column (`crate::fold`). Searches use
    // `fold_has`; this one stays for the oracle in the differential test.
    conn.create_scalar_function(
        "fold",
        1,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |ctx| {
            Ok(match ctx.get_raw(0) {
                ValueRef::Text(t) => Some(search_fold(&String::from_utf8_lossy(t)).into_owned()),
                _ => None,
            })
        },
    )?;
    // `fold_has(needle, col...)`: 1 when the folded needle is inside the
    // folded form of any text column. Same match as `fold(col) LIKE
    // '%needle%'`, without building a folded copy of every column of every
    // row: a column with no capital or non-ASCII byte is searched in place,
    // others fold into a reused buffer, and the first hit ends the row.
    conn.create_scalar_function(
        "fold_has",
        -1,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |ctx| {
            let ValueRef::Text(needle) = ctx.get_raw(0) else {
                return Ok(false);
            };
            let needle = String::from_utf8_lossy(needle);
            FOLD_BUF.with(|buf| {
                let buf = &mut *buf.borrow_mut();
                Ok((1..ctx.len()).any(|i| match ctx.get_raw(i) {
                    ValueRef::Text(t) => folded_contains(t, &needle, buf),
                    _ => false,
                }))
            })
        },
    )?;
    Ok(())
}

thread_local! {
    /// The scratch buffer of `fold_has`, one per thread that runs queries.
    static FOLD_BUF: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// Whether the folded form of the text `hay` contains `needle` (already
/// folded). Invalid UTF-8 is read as `fold` read it: lossily.
fn folded_contains(hay: &[u8], needle: &str, buf: &mut String) -> bool {
    match std::str::from_utf8(hay) {
        Ok(s) if !needs_fold(s) => s.contains(needle),
        Ok(s) => {
            fold_into(s, buf);
            buf.contains(needle)
        }
        Err(_) => {
            fold_into(&String::from_utf8_lossy(hay), buf);
            buf.contains(needle)
        }
    }
}

fn user_version(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// The needle of a search: folded like the columns it is matched against
/// (`fold_has`).
fn search_needle(search: &str) -> String {
    search_fold(search.trim()).into_owned()
}

/// The search clause of the Drafts and Outbox views: `?1` is the search
/// (matching everything when empty), `?2` its folded needle.
const DRAFT_MATCH: &str = "(?1 = '' OR fold_has(?2, subject, to_addrs, cc_addrs, bcc_addrs, body))";

/// The list query of [`Db::message_ids_filtered`]; a search binds its
/// folded needle to `?1`.
fn list_query(folder_ids: &[i64], search: &str, filter: Filter) -> String {
    let ids = folder_ids
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    // Left to itself, SQLite walks the whole folder in date order and
    // reads every row's flag; the unread few are faster found and sorted.
    let (index, flags) = match filter {
        Filter::All => ("", ""),
        Filter::Unread => ("INDEXED BY messages_folder_seen", "AND seen = 0"),
        Filter::Flagged => ("", "AND flagged = 1"),
    };
    let matching = if search.trim().is_empty() {
        ""
    } else {
        "AND fold_has(?1, subject, from_name, from_addr, preview, to_addrs, cc_addrs)"
    };
    format!(
        "SELECT id FROM messages {index} WHERE folder_id IN ({ids}) {flags} {matching}
         ORDER BY date DESC, id DESC"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The search clause before `fold_has`, kept as the oracle: six
    /// `fold(col) LIKE` terms.
    fn like_ids(db: &Db, cols: &[&str], table_where: &str, query: &str) -> Vec<i64> {
        let pattern = format!(
            "%{}%",
            search_fold(query.trim())
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let terms = cols
            .iter()
            .map(|c| format!("fold({c}) LIKE ?1 ESCAPE '\\'"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let sql = format!("SELECT id FROM {table_where} AND ({terms}) ORDER BY id");
        let mut stmt = db.conn.prepare(&sql).unwrap();
        stmt.query_map([pattern], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    /// Known divergence, decided: SQLite's LIKE reads a text as a C string,
    /// so the old terms never saw past an embedded NUL; `fold_has` does. The
    /// old ids must all be found, and any extra id has a NUL in a column.
    fn same_but_after_nul(db: &Db, table: &str, cols: &[&str], got: &[i64], want: &[i64], q: &str) {
        assert!(
            want.iter().all(|i| got.contains(i)),
            "{table}, query {q:?}: lost a match"
        );
        let blob = cols
            .iter()
            .map(|c| format!("CAST({c} AS BLOB)"))
            .collect::<Vec<_>>()
            .join(" || ");
        for id in got.iter().filter(|i| !want.contains(i)) {
            let nul: bool = db
                .conn
                .query_row(
                    &format!("SELECT instr({blob}, X'00') > 0 FROM {table} WHERE id = ?1"),
                    [id],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(nul, "{table}, query {q:?}: extra match {id} without a NUL");
        }
    }

    /// `fold_has` finds the rows the old `fold(col) LIKE` terms did:
    /// accents, case, LIKE metacharacters, NUL, invalid UTF-8, non-text
    /// columns, NULL and the empty needle (docs/PERF.md 3.1, 3.7).
    #[test]
    fn fold_has_matches_the_like_terms() {
        let db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        let pieces = [
            "a",
            "A",
            "\u{e9}",
            "\u{c9}",
            "e\u{301}",
            "\u{df}",
            "SS",
            "ss",
            "\u{130}",
            "i",
            "_",
            "%",
            "\\",
            "x",
            "\u{0}",
            "Jo\u{e3}o",
            "joao",
            " ",
            "@",
            "\u{6771}",
            "\u{3a3}",
            "\u{3c3}",
            "\u{1f642}",
        ];
        let mut seed = 0x2545_f491_u64;
        let mut next = move |n: usize| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            usize::try_from(seed >> 33).unwrap_or(0) % n
        };
        let word = |next: &mut dyn FnMut(usize) -> usize| -> String {
            (0..next(6)).map(|_| pieces[next(pieces.len())]).collect()
        };
        for uid in 1..=300_i64 {
            let cols: Vec<String> = (0..6).map(|_| word(&mut next)).collect();
            db.conn
                .execute(
                    "INSERT INTO messages (account_id, folder_id, uid, subject, from_name, from_addr,
                                           preview, to_addrs, cc_addrs)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![acc, f, uid, cols[0], cols[1], cols[2], cols[3], cols[4], cols[5]],
                )
                .unwrap();
            db.conn
                .execute(
                    "INSERT INTO drafts (account_id, subject, to_addrs, cc_addrs, bcc_addrs, body,
                                         message_id, state, created, updated)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'm', ?7, 0, ?3)",
                    params![acc, cols[0], cols[1], cols[2], cols[3], cols[4], uid % 2],
                )
                .unwrap();
        }
        // Invalid UTF-8 text, a blob, an integer and a real in searched columns.
        db.conn
            .execute_batch(
                "UPDATE messages SET subject = CAST(X'61ff4a6f' AS TEXT) WHERE uid = 1;
                 UPDATE messages SET from_name = X'6a6f616f' WHERE uid = 2;
                 UPDATE messages SET from_addr = 12345 WHERE uid = 3;
                 UPDATE messages SET preview = 2.5 WHERE uid = 4;
                 UPDATE drafts SET body = CAST(X'c4b0ff' AS TEXT) WHERE id = 1;",
            )
            .unwrap();
        let mut queries: Vec<String> = [
            "",
            " ",
            "a",
            "A",
            "joao",
            "JO\u{c3}O",
            "\u{e9}",
            "e",
            "ss",
            "STRASSE",
            "_",
            "%",
            "\\",
            "a_",
            "%a",
            "\u{0}",
            "x",
            "i",
            "\u{130}",
            "12345",
            "2.5",
            "\u{fffd}",
            "@",
            "zzzz",
            "\u{3c3}\u{3c3}",
            "\u{1f642}",
        ]
        .iter()
        .map(|q| q.to_string())
        .collect();
        for _ in 0..80 {
            queries.push(word(&mut next));
        }
        let msg_cols = [
            "subject",
            "from_name",
            "from_addr",
            "preview",
            "to_addrs",
            "cc_addrs",
        ];
        let draft_cols = ["subject", "to_addrs", "cc_addrs", "bcc_addrs", "body"];
        let mut hits = 0;
        for q in queries.iter().filter(|q| !q.contains(' ')) {
            let want = like_ids(&db, &msg_cols, "messages WHERE 1", q);
            let mut got = db.message_ids(&[f], q).unwrap();
            got.sort_unstable();
            if q.trim().is_empty() {
                assert_eq!(got.len(), 300, "empty search lists everything");
            } else {
                same_but_after_nul(&db, "messages", &msg_cols, &got, &want, q);
                hits += want.len();
            }
            for (state, ids) in [
                ("= 0", db.draft_ids(q).unwrap()),
                ("> 0", db.outbox_ids(q).unwrap()),
            ] {
                let mut got = ids;
                got.sort_unstable();
                if q.trim().is_empty() {
                    assert_eq!(got.len(), 150);
                } else {
                    let want =
                        like_ids(&db, &draft_cols, &format!("drafts WHERE state {state}"), q);
                    same_but_after_nul(&db, "drafts", &draft_cols, &got, &want, q);
                }
            }
        }
        assert!(
            hits > 100,
            "the corpus must actually match something: {hits}"
        );
        // NULL and non-text arguments never match; no columns match nothing.
        let one = |sql: &str| -> i64 { db.conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(one("SELECT fold_has('a', NULL, 7, X'61')"), 0);
        assert_eq!(one("SELECT fold_has('a', NULL, 'bAb')"), 1);
        assert_eq!(one("SELECT fold_has('a')"), 0);
        assert_eq!(one("SELECT fold_has(NULL, 'a')"), 0);
    }

    fn account(db: &Db) -> i64 {
        db.add_account(&NewAccount {
            email: "me@example.com".into(),
            name: "Me".into(),
            imap: Server {
                host: "imap.example.com".into(),
                port: 993,
                security: Security::Tls,
            },
            smtp: Server {
                host: "smtp.example.com".into(),
                port: 587,
                security: Security::StartTls,
            },
            save_sent: true,
            imap_user: String::new(),
            smtp_user: String::new(),
        })
        .unwrap()
    }

    fn header(uid: u32, date: i64, subject: &str) -> NewHeader {
        NewHeader {
            uid,
            subject: subject.into(),
            from_name: "Jane".into(),
            from_addr: "jane@example.com".into(),
            date,
            ..Default::default()
        }
    }

    #[test]
    fn account_roundtrip() {
        let db = Db::in_memory().unwrap();
        let id = account(&db);
        let a = db.accounts().unwrap().remove(0);
        assert_eq!(a.id, id);
        assert_eq!(a.email, "me@example.com");
        assert_eq!(a.smtp.security, Security::StartTls);
        assert!(db.add_account(&NewAccount { ..a_to_new(&a) }).is_err());
        db.remove_account(id).unwrap();
        assert!(db.accounts().unwrap().is_empty());
    }

    fn a_to_new(a: &Account) -> NewAccount {
        NewAccount {
            email: a.email.clone(),
            name: a.name.clone(),
            imap: a.imap.clone(),
            smtp: a.smtp.clone(),
            save_sent: a.save_sent,
            imap_user: a.imap_user.clone(),
            smtp_user: a.smtp_user.clone(),
        }
    }

    #[test]
    fn server_copies_of_drafts_are_found_by_folder_uid_and_validity() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let drafts = db
            .upsert_folder(acc, "Drafts", "Drafts", 0, Role::Drafts)
            .unwrap();
        let inbox = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        db.set_uidvalidity(drafts, 7).unwrap();
        db.insert_headers(
            acc,
            drafts,
            &[header(1, 10, "mine"), header(2, 20, "webmail")],
        )
        .unwrap();
        db.insert_headers(acc, inbox, &[header(1, 30, "same uid, other folder")])
            .unwrap();
        let ids = db.message_ids(&[drafts], "").unwrap();
        let (webmail, mine) = (ids[0], ids[1]);
        assert!(db.draft_copies(&[drafts]).unwrap().is_empty());
        let row = db
            .insert_draft(&DraftRow {
                account_id: acc,
                subject: "mine".into(),
                message_id: "<m@x>".into(),
                ..Default::default()
            })
            .unwrap();
        let copy = |uidvalidity| ServerCopy {
            account_id: acc,
            folder_id: drafts,
            uidvalidity,
            uid: 1,
        };
        db.record_server_copy(row, 1, Some(&copy(7))).unwrap();
        // Its copy, never the webmail draft nor the Inbox's uid 1.
        assert_eq!(
            db.draft_copies(&[drafts, inbox]).unwrap(),
            HashSet::from([mine])
        );
        assert!(db.draft_copies(&[inbox]).unwrap().is_empty());
        assert!(!db.draft_copies(&[drafts]).unwrap().contains(&webmail));
        // A copy recorded under another UIDVALIDITY names nothing here.
        db.record_server_copy(row, 2, Some(&copy(6))).unwrap();
        assert!(db.draft_copies(&[drafts]).unwrap().is_empty());
    }

    #[test]
    fn view_and_fold_settings_round_trip() {
        let db = Db::in_memory().unwrap();
        assert_eq!(db.settings().unwrap().last_view, "");
        db.set_setting("last_view", "f12").unwrap();
        db.set_setting("nav_folds_init", "1,3").unwrap();
        let s = db.settings().unwrap();
        assert_eq!(
            (s.last_view.as_str(), s.nav_folds_init.as_str()),
            ("f12", "1,3")
        );
    }

    #[test]
    fn headers_sorted_and_searchable() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        db.insert_headers(
            acc,
            f,
            &[
                header(1, 100, "Old news"),
                header(2, 300, "Invoice 100%"),
                header(3, 200, "Lunch?"),
            ],
        )
        .unwrap();
        let subjects = |search: &str| -> Vec<String> {
            db.message_ids(&[f], search)
                .unwrap()
                .into_iter()
                .map(|id| db.header(id).unwrap().unwrap().subject)
                .collect()
        };
        assert_eq!(subjects(""), ["Invoice 100%", "Lunch?", "Old news"]);
        assert_eq!(subjects("lunch").len(), 1);
        assert_eq!(subjects("100%").len(), 1);
        assert_eq!(subjects("jane").len(), 3);
        // Inserting the same UID again is ignored.
        db.insert_headers(acc, f, &[header(1, 999, "dup")]).unwrap();
        assert_eq!(db.message_ids(&[f], "").unwrap().len(), 3);
    }

    #[test]
    fn search_ignores_case_and_accents() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let inbox = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        let archive = db
            .upsert_folder(acc, "Archive", "Archive", 0, Role::Archive)
            .unwrap();
        let sent = db
            .upsert_folder(acc, "Sent", "Sent", 0, Role::Sent)
            .unwrap();
        db.insert_headers(
            acc,
            inbox,
            &[
                header(1, 100, "AÇÃO urgente"),
                header(2, 200, "Straße 5"),
                NewHeader {
                    cc: "Ana Núñez <ana@example.com>".into(),
                    ..header(3, 300, "Lunch")
                },
            ],
        )
        .unwrap();
        db.insert_headers(
            acc,
            archive,
            &[NewHeader {
                from_name: "João Gonçalves".into(),
                from_addr: "joao@example.com".into(),
                ..header(1, 50, "Orçamento")
            }],
        )
        .unwrap();
        db.insert_headers(
            acc,
            sent,
            &[NewHeader {
                to: "JOSÉ <jose@example.com>".into(),
                ..header(1, 60, "Re: Orçamento")
            }],
        )
        .unwrap();
        let subjects = |folders: &[i64], search: &str| -> Vec<String> {
            db.message_ids(folders, search)
                .unwrap()
                .into_iter()
                .map(|id| db.header(id).unwrap().unwrap().subject)
                .collect()
        };
        assert_eq!(subjects(&[inbox], "acao"), ["AÇÃO urgente"]);
        assert_eq!(subjects(&[inbox], "Ação"), ["AÇÃO urgente"]);
        assert_eq!(subjects(&[inbox], "STRASSE"), ["Straße 5"]);
        assert_eq!(subjects(&[inbox], "strasse"), ["Straße 5"]);
        // Cc (and To) are searched too.
        assert_eq!(subjects(&[inbox], "nunez"), ["Lunch"]);
        // "joao" finds "João", but only where it is searched.
        assert!(subjects(&[inbox], "joao").is_empty());
        assert_eq!(subjects(&[archive], "joao"), ["Orçamento"]);
        // All folders: the Archive and the Sent rows (by recipient).
        let all = [inbox, archive, sent];
        assert_eq!(subjects(&all, "orcamento"), ["Re: Orçamento", "Orçamento"]);
        assert_eq!(subjects(&all, "jose"), ["Re: Orçamento"]);
        assert_eq!(subjects(&all, "  GONÇALVES "), ["Orçamento"]);

        // The SQL function takes anything without failing.
        let odd: (Option<String>, Option<String>, Option<String>) = db
            .conn
            .query_row(
                "SELECT fold(NULL), fold(42), fold(CAST(x'ff4a' AS TEXT))",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(odd, (None, None, Some("\u{fffd}j".into())));
        db.checkpoint();
        db.optimize();
    }

    #[test]
    fn search_escapes_like_characters() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        db.insert_headers(
            acc,
            f,
            &[
                header(1, 100, "Invoice 100%"),
                header(2, 200, "Invoice 1000"),
                header(3, 300, "file_name"),
                header(4, 400, "filename"),
                header(5, 500, r"C:\Temp"),
            ],
        )
        .unwrap();
        let subjects = |search: &str| -> Vec<String> {
            db.message_ids(&[f], search)
                .unwrap()
                .into_iter()
                .map(|id| db.header(id).unwrap().unwrap().subject)
                .collect()
        };
        assert_eq!(subjects("100%"), ["Invoice 100%"]);
        assert_eq!(subjects("%"), ["Invoice 100%"]);
        assert_eq!(subjects("file_"), ["file_name"]);
        assert_eq!(subjects("_"), ["file_name"]);
        assert_eq!(subjects(r"c:\t"), [r"C:\Temp"]);
        assert_eq!(subjects(r"\"), [r"C:\Temp"]);
    }

    #[test]
    fn draft_search_ignores_case_and_accents() {
        let db = Db::in_memory().unwrap();
        let acc = account(&db);
        let id = db
            .insert_draft(&DraftRow {
                subject: "Orçamento".into(),
                to: "João <joao@x.org>".into(),
                ..draft(acc)
            })
            .unwrap();
        assert_eq!(db.draft_ids("ORCAMENTO").unwrap(), [id]);
        assert_eq!(db.draft_ids("joao").unwrap(), [id]);
        assert!(db.draft_ids("100%").unwrap().is_empty());
    }

    #[test]
    fn filters_keep_unread_or_flagged_messages() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        let other = db
            .upsert_folder(acc, "Work", "Work", 0, Role::Other)
            .unwrap();
        let mut headers: Vec<NewHeader> = (1..=6)
            .map(|uid| header(uid, i64::from(uid) * 100, &format!("m{uid}")))
            .collect();
        for h in &mut headers {
            h.seen = h.uid % 2 == 0;
            h.flagged = h.uid >= 5;
        }
        db.insert_headers(acc, f, &headers).unwrap();
        db.insert_headers(acc, other, &[header(1, 50, "elsewhere")])
            .unwrap();
        let subjects = |folders: &[i64], search: &str, filter| -> Vec<String> {
            db.message_ids_filtered(folders, search, filter)
                .unwrap()
                .into_iter()
                .map(|id| db.header(id).unwrap().unwrap().subject)
                .collect()
        };
        assert_eq!(subjects(&[f], "", Filter::All).len(), 6);
        assert_eq!(subjects(&[f], "", Filter::Unread), ["m5", "m3", "m1"]);
        assert_eq!(subjects(&[f], "", Filter::Flagged), ["m6", "m5"]);
        assert_eq!(subjects(&[f], "m3", Filter::Unread), ["m3"]);
        assert!(subjects(&[f], "m3", Filter::Flagged).is_empty());
        assert_eq!(
            subjects(&[f, other], "", Filter::Unread),
            ["m5", "m3", "m1", "elsewhere"]
        );
    }

    #[test]
    fn the_unread_filter_reads_the_seen_index() {
        let db = Db::in_memory().unwrap();
        let plan = |folders: &[i64], filter| -> String {
            let sql = list_query(folders, "", filter);
            let mut stmt = db
                .conn
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .unwrap();
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(3))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows.join("; ")
        };
        for folders in [&[3][..], &[3, 4]] {
            let unread = plan(folders, Filter::Unread);
            assert!(unread.contains("messages_folder_seen"), "{unread}");
        }
        // Everything else lists in index order, without a sort.
        let all = plan(&[3], Filter::All);
        assert!(all.contains("messages_folder_date"), "{all}");
        assert!(!all.contains("TEMP B-TREE"), "{all}");
    }

    #[test]
    fn mark_folder_read_reports_the_unread_uids_and_the_range() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        let other = db
            .upsert_folder(acc, "Work", "Work", 0, Role::Other)
            .unwrap();
        assert_eq!(db.mark_folder_read(f).unwrap(), (vec![], 0));
        let mut headers: Vec<NewHeader> = [3, 7, 9]
            .into_iter()
            .map(|uid| header(uid, i64::from(uid), "m"))
            .collect();
        headers[2].seen = true;
        db.insert_headers(acc, f, &headers).unwrap();
        db.insert_headers(acc, other, &[header(4, 1, "w")]).unwrap();
        let (mut uids, highest) = db.mark_folder_read(f).unwrap();
        uids.sort_unstable();
        assert_eq!((uids, highest), (vec![3, 7], 9));
        assert!(
            db.message_ids_filtered(&[f], "", Filter::Unread)
                .unwrap()
                .is_empty()
        );
        // Other folders keep their unread mail.
        assert_eq!(db.unread_counts().unwrap().get(&other), Some(&1));
    }

    #[test]
    fn flags_and_deletions_follow_server() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        db.insert_headers(
            acc,
            f,
            &[header(1, 1, "a"), header(2, 2, "b"), header(3, 3, "c")],
        )
        .unwrap();
        assert_eq!(db.unread_counts().unwrap()[&f], 3);
        // UID 2 was just marked read locally and the change is still in
        // flight, so the server's stale "unseen" must not undo it.
        let b = db.message_ids(&[f], "").unwrap()[1];
        db.set_seen(b, true).unwrap();
        // Server: 1 seen+flagged, 2 and 3 unseen.
        let remote = HashMap::from([(1, SEEN | FLAGGED), (2, 0), (3, 0)]);
        let changed = db.apply_flags(f, &remote, &HashSet::from([2])).unwrap();
        assert_eq!(changed, 1);
        assert_eq!(db.unread_counts().unwrap()[&f], 1);
        assert!(db.message(b).unwrap().unwrap().seen);
        db.delete_uids(f, &[3]).unwrap();
        assert_eq!(db.known_uids(f).unwrap(), HashSet::from([1, 2]));
        assert!(!db.unread_counts().unwrap().contains_key(&f));
    }

    #[test]
    fn flag_delta_touches_only_listed_uids() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        db.insert_headers(acc, f, &[header(1, 1, "a"), header(2, 2, "b")])
            .unwrap();
        // UID 1 unchanged, 3 not cached, 2 busy: nothing to update.
        let delta = HashMap::from([(1, 0), (2, SEEN), (3, SEEN)]);
        assert_eq!(
            db.apply_flag_delta(f, &delta, &HashSet::from([2])).unwrap(),
            0
        );
        let delta = HashMap::from([(2, SEEN | ANSWERED)]);
        assert_eq!(db.apply_flag_delta(f, &delta, &HashSet::new()).unwrap(), 1);
        assert_eq!(db.unread_counts().unwrap()[&f], 1);
    }

    #[test]
    fn uidvalidity_change_resets_folder() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        assert!(!db.set_uidvalidity(f, 7).unwrap());
        db.insert_headers(acc, f, &[header(1, 1, "a")]).unwrap();
        assert!(!db.set_uidvalidity(f, 7).unwrap());
        assert_eq!(db.known_uids(f).unwrap().len(), 1);
        let fp = Fp {
            uidnext: 2,
            messages: 1,
            hms: 1 << 40,
        };
        db.finish_folder(f, fp).unwrap();
        assert_eq!(db.fingerprint(f).unwrap(), Some(fp));
        assert_eq!(db.count_messages(f).unwrap(), 1);
        assert!(db.set_uidvalidity(f, 8).unwrap());
        assert!(db.known_uids(f).unwrap().is_empty());
        assert_eq!(db.fingerprint(f).unwrap(), None);
        db.finish_folder(f, fp).unwrap();
        db.clear_fingerprint(f).unwrap();
        assert_eq!(db.fingerprint(f).unwrap(), None);
    }

    /// A unique file under the temp directory, removed (with its WAL files)
    /// on drop.
    struct TempDb(PathBuf);

    impl TempDb {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "komorebi-test-{}-{name}.sqlite3",
                std::process::id()
            ));
            let t = TempDb(path);
            t.remove();
            t
        }

        fn remove(&self) {
            for suffix in ["", "-wal", "-shm"] {
                let mut p = self.0.clone().into_os_string();
                p.push(suffix);
                std::fs::remove_file(p).ok();
            }
            std::fs::remove_dir_all(drafts_dir(&self.0)).ok();
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            self.remove();
        }
    }

    /// What v5 added.
    const DROP_V5: &str = "DROP TABLE draft_attachments; DROP TABLE drafts;";
    /// Undo v7 (after v8).
    const DROP_V7: &str = "ALTER TABLE accounts DROP COLUMN signature; DROP TABLE settings;";
    /// Undo v8 (after v9).
    const DROP_V8: &str = "DROP TABLE addresses; ALTER TABLE accounts DROP COLUMN addr_backfill;";
    /// Undo v12 (run first: it is the newest).
    const DROP_V12: &str = "ALTER TABLE accounts DROP COLUMN imap_user;
        ALTER TABLE accounts DROP COLUMN smtp_user; ALTER TABLE accounts DROP COLUMN imap_pin;
        ALTER TABLE accounts DROP COLUMN smtp_pin; ALTER TABLE folders DROP COLUMN role_override;";
    /// Undo v11 (after v12).
    const DROP_V11: &str = "ALTER TABLE accounts DROP COLUMN enabled;";
    /// Undo v10 (after v11).
    const DROP_V10: &str = "DROP TABLE rsvp;";
    /// Undo v9 (after v10).
    const DROP_V9: &str = "ALTER TABLE drafts DROP COLUMN rev; ALTER TABLE drafts DROP COLUMN server_rev;
        ALTER TABLE drafts DROP COLUMN server_folder; ALTER TABLE drafts DROP COLUMN server_uidvalidity;
        ALTER TABLE drafts DROP COLUMN server_uid;";

    /// Turn a current database back into v2 (no fingerprint columns).
    fn downgrade_to_v2(conn: &Connection) {
        conn.execute_batch(&format!(
            "{DROP_V12} {DROP_V11} {DROP_V10} {DROP_V9} {DROP_V8} {DROP_V7}
             ALTER TABLE folders DROP COLUMN sync_uidnext;
             ALTER TABLE folders DROP COLUMN sync_messages;
             ALTER TABLE folders DROP COLUMN sync_hms;
             DROP TABLE remote_allow;
             {DROP_V5}
             PRAGMA user_version = 2;"
        ))
        .unwrap();
    }

    #[test]
    fn batch_flags_and_deletes() {
        let mut db = Db::in_memory().unwrap();
        let account = account(&db);
        let inbox = db
            .upsert_folder(account, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        let headers: Vec<_> = (1..=1200).map(|uid| header(uid, uid as i64, "s")).collect();
        db.insert_headers(account, inbox, &headers).unwrap();
        let ids = db.message_ids(&[inbox], "").unwrap();
        assert_eq!(ids.len(), 1200);

        // More ids than one query takes, plus one that isn't cached.
        let mut asked = ids.clone();
        asked.push(-1);
        let briefs = db.briefs(&asked).unwrap();
        assert_eq!(briefs.len(), 1200);
        assert!(
            briefs
                .iter()
                .all(|b| b.folder_id == inbox && !b.seen && !b.flagged)
        );

        db.set_seen_many(&ids[..700], true).unwrap();
        db.set_flagged_many(&ids[..3], true).unwrap();
        assert_eq!(db.unread_counts().unwrap()[&inbox], 500);
        let flagged = db.briefs(&ids[..4]).unwrap();
        assert_eq!(flagged.iter().filter(|b| b.flagged).count(), 3);

        db.delete_messages(&ids[100..]).unwrap();
        assert_eq!(db.message_ids(&[inbox], "").unwrap(), ids[..100]);
        assert!(db.briefs(&[]).unwrap().is_empty());
    }

    #[test]
    fn a_damaged_cache_is_set_aside_and_a_new_one_made() {
        let dir = std::env::temp_dir().join(format!("komorebi-damaged-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("cache.drafts").join("1")).unwrap();
        let path = dir.join("cache.sqlite3");
        std::fs::write(&path, b"this is not a database, just bytes".repeat(200)).unwrap();
        std::fs::write(dir.join("cache.sqlite3-wal"), b"stale log").unwrap();

        let (db, aside) = Db::open_ui(&path).unwrap();
        let aside = aside.expect("set aside");
        let name = aside.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("cache.damaged-") && name.ends_with(".sqlite3"),
            "{name}"
        );
        // The old bytes are kept, with the drafts folder.
        assert!(std::fs::read(&aside).unwrap().starts_with(b"this is not"));
        assert!(drafts_dir(&aside).join("1").exists());
        assert!(!dir.join("cache.drafts").exists());
        // A new, empty cache works.
        assert!(db.accounts().unwrap().is_empty());
        assert_eq!(user_version(&db.conn).unwrap(), SCHEMA_VERSION);
        drop(db);
        // Opened again, nothing more is set aside.
        let (_, again) = Db::open_ui(&path).unwrap();
        assert!(again.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_newer_cache_is_refused_and_left_alone() {
        let dir = std::env::temp_dir().join(format!("komorebi-newer-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cache.sqlite3");
        let db = Db::open(&path).unwrap();
        db.conn
            .execute_batch(&format!("PRAGMA user_version = {};", SCHEMA_VERSION + 1))
            .unwrap();
        drop(db);
        let e = Db::open_ui(&path).err().expect("refused");
        assert!(e.downcast_ref::<NewerCache>().is_some());
        assert!(format!("{e:#}").starts_with("This cache was made by a newer Komorebi"));
        // Nothing was renamed or reset.
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.contains("damaged")), "{names:?}");
        let conn = Connection::open(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION + 1);
        drop(conn);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn migrates_v3_to_v4() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        db.insert_headers(acc, f, &[header(1, 1, "a")]).unwrap();
        let id = db.message_ids(&[f], "").unwrap()[0];
        db.set_body(id, b"raw", "preview", false).unwrap();
        db.conn
            .execute_batch(&format!(
                "{DROP_V12} {DROP_V11} {DROP_V10} {DROP_V9} {DROP_V8} {DROP_V7} DROP TABLE remote_allow; {DROP_V5} PRAGMA user_version = 3;"
            ))
            .unwrap();

        let db = Db::init(db.conn).unwrap();
        assert_eq!(user_version(&db.conn).unwrap(), SCHEMA_VERSION);
        assert!(db.remote_allow_list().unwrap().is_empty());
        db.allow_remote("news@shop.example").unwrap();
        assert!(db.remote_allowed("news@shop.example").unwrap());
        assert_eq!(
            db.message(id).unwrap().unwrap().raw.as_deref(),
            Some(&b"raw"[..])
        );
    }

    #[test]
    fn remote_allow_keys_on_the_exact_address() {
        let db = Db::in_memory().unwrap();
        db.allow_remote(" News@Shop.Example ").unwrap();
        db.allow_remote("news@shop.example").unwrap();
        assert_eq!(db.remote_allow_list().unwrap(), ["news@shop.example"]);
        assert!(db.remote_allowed("NEWS@shop.example").unwrap());
        // Not the domain, another address at it, or a display name that
        // spells the allowed address in front of another one.
        for other in [
            "shop.example",
            "@shop.example",
            "billing@shop.example",
            "news@shop.example.evil.test",
            "news@shop.example <evil@evil.test>",
            "\"news@shop.example\" <evil@evil.test>",
            "evil@evil.test",
            "",
        ] {
            assert!(!db.remote_allowed(other).unwrap(), "{other}");
        }
        // A message's sender is matched on its From address alone.
        let mut db = db;
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        let spoof = NewHeader {
            uid: 1,
            from_name: "news@shop.example".into(),
            from_addr: "evil@evil.test".into(),
            ..Default::default()
        };
        db.insert_headers(acc, f, &[spoof]).unwrap();
        let id = db.message_ids(&[f], "").unwrap()[0];
        let m = db.message(id).unwrap().unwrap();
        assert!(!db.remote_allowed(&m.from_addr).unwrap());
        assert!(db.allow_remote("Shop <news@shop.example>").is_err());

        db.allow_remote("other@x.example").unwrap();
        db.forget_remote("NEWS@shop.example").unwrap();
        assert_eq!(db.remote_allow_list().unwrap(), ["other@x.example"]);
        db.forget_all_remote().unwrap();
        assert!(db.remote_allow_list().unwrap().is_empty());
    }

    #[test]
    fn migrates_v2_to_v3() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        db.set_uidvalidity(f, 7).unwrap();
        db.insert_headers(acc, f, &[header(1, 1, "a")]).unwrap();
        let id = db.message_ids(&[f], "").unwrap()[0];
        db.set_body(id, b"raw", "preview", false).unwrap();
        downgrade_to_v2(&db.conn);

        let db = Db::init(db.conn).unwrap();
        assert_eq!(user_version(&db.conn).unwrap(), SCHEMA_VERSION);
        assert_eq!(db.fingerprint(f).unwrap(), None);
        let nulls: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM folders WHERE sync_uidnext IS NULL
                    AND sync_messages IS NULL AND sync_hms IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(nulls, 1);
        assert_eq!(db.folder(f).unwrap().unwrap().uidvalidity, 7);
        assert_eq!(
            db.message(id).unwrap().unwrap().raw.as_deref(),
            Some(&b"raw"[..])
        );
    }

    #[test]
    fn unknown_schema_version_is_refused() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA user_version = 99;").unwrap();
        assert!(Db::init(conn).is_err());
    }

    #[test]
    fn worker_refuses_unmigrated_cache() {
        let tmp = TempDb::new("worker-v2");
        // A missing file is not created.
        assert!(Db::open_worker(&tmp.0).is_err());
        assert!(!tmp.0.exists());
        {
            let db = Db::open(&tmp.0).unwrap();
            downgrade_to_v2(&db.conn);
        }
        assert!(Db::open_worker(&tmp.0).is_err());
        // Once the UI connection migrates it, workers can open it.
        drop(Db::open_ui(&tmp.0).unwrap());
        assert!(Db::open_worker(&tmp.0).is_ok());
    }

    #[test]
    fn writer_waits_for_another_connections_write_lock() {
        let tmp = TempDb::new("busy");
        let (mut db, _) = Db::open_ui(&tmp.0).unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        let other = Db::open_worker(&tmp.0).unwrap();
        other.conn.execute_batch("BEGIN IMMEDIATE;").unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            other.conn.execute_batch("COMMIT;").unwrap();
        });
        let start = std::time::Instant::now();
        db.insert_headers(acc, f, &[header(1, 1, "a")]).unwrap();
        assert!(start.elapsed() >= Duration::from_millis(50));
        release.join().unwrap();
        assert_eq!(db.count_messages(f).unwrap(), 1);
    }

    #[test]
    fn folders_ordered_by_role_and_retained() {
        let db = Db::in_memory().unwrap();
        let acc = account(&db);
        let zed = db.upsert_folder(acc, "Zed", "Zed", 0, Role::Other).unwrap();
        let trash = db
            .upsert_folder(acc, "Trash", "Trash", 0, Role::Trash)
            .unwrap();
        let inbox = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        let ids: Vec<i64> = db.folders().unwrap().iter().map(|f| f.id).collect();
        assert_eq!(ids, [inbox, trash, zed]);
        db.retain_folders(acc, &[inbox, trash]).unwrap();
        assert_eq!(db.folders().unwrap().len(), 2);
        assert_eq!(
            db.folder_by_role(acc, Role::Trash).unwrap().unwrap().id,
            trash
        );
    }

    #[test]
    fn single_folder_list_needs_no_sort() {
        let db = Db::in_memory().unwrap();
        let plan: Vec<String> = db
            .conn
            .prepare(
                "EXPLAIN QUERY PLAN SELECT id FROM messages WHERE folder_id IN (1)
                 ORDER BY date DESC, id DESC",
            )
            .unwrap()
            .query_map([], |r| r.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(!plan.iter().any(|p| p.contains("TEMP B-TREE")), "{plan:?}");
    }

    /// Every table's columns and foreign keys and every index's columns.
    fn shape(conn: &Connection) -> Vec<String> {
        let names: Vec<(String, String)> = conn
            .prepare(
                "SELECT type, name FROM sqlite_master
                 WHERE name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let rows = |sql: &str| -> Vec<String> {
            let mut stmt = conn.prepare(sql).unwrap();
            let n = stmt.column_count();
            stmt.query_map([], |r| {
                Ok((0..n)
                    .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                    .collect::<Vec<_>>()
                    .join("|"))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
        };
        let mut out = Vec::new();
        for (kind, name) in names {
            out.push(format!("{kind} {name}"));
            if kind == "table" {
                out.extend(rows(&format!("PRAGMA table_info({name})")));
                out.extend(rows(&format!("PRAGMA foreign_key_list({name})")));
            } else {
                out.extend(rows(&format!("PRAGMA index_xinfo({name})")));
            }
        }
        out
    }

    #[test]
    fn a_migrated_cache_matches_a_new_one() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(V1_CACHE).unwrap();
        let migrated = Db::init(conn).unwrap();
        let new = Db::in_memory().unwrap();
        assert_eq!(shape(&migrated.conn), shape(&new.conn));
    }

    #[test]
    fn a_v11_cache_migrates_to_the_new_shape() {
        let old = Db::in_memory().unwrap();
        old.conn
            .execute_batch(&format!("{DROP_V12} PRAGMA user_version = 11;"))
            .unwrap();
        let migrated = Db::init(old.conn).unwrap();
        let new = Db::in_memory().unwrap();
        let columns = |db: &Db, table: &str| -> Vec<String> {
            db.conn
                .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        assert_eq!(columns(&migrated, "accounts"), columns(&new, "accounts"));
        assert_eq!(columns(&migrated, "folders"), columns(&new, "folders"));
        assert_eq!(shape(&migrated.conn), shape(&new.conn));
    }

    /// A cache as v1 wrote it (bodies inline in `messages`).
    const V1_CACHE: &str =
        "CREATE TABLE accounts (id INTEGER PRIMARY KEY, email TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL DEFAULT '', imap_host TEXT NOT NULL, imap_port INTEGER NOT NULL,
            imap_tls INTEGER NOT NULL, smtp_host TEXT NOT NULL, smtp_port INTEGER NOT NULL,
            smtp_tls INTEGER NOT NULL, save_sent INTEGER NOT NULL DEFAULT 1,
            position INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE folders (id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL
            REFERENCES accounts(id) ON DELETE CASCADE, name TEXT NOT NULL, display TEXT NOT NULL,
            depth INTEGER NOT NULL DEFAULT 0, role TEXT NOT NULL DEFAULT '',
            uidvalidity INTEGER NOT NULL DEFAULT 0, UNIQUE(account_id, name));
         CREATE TABLE messages (id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL
            REFERENCES accounts(id) ON DELETE CASCADE, folder_id INTEGER NOT NULL
            REFERENCES folders(id) ON DELETE CASCADE, uid INTEGER NOT NULL,
            msgid TEXT NOT NULL DEFAULT '', subject TEXT NOT NULL DEFAULT '',
            from_name TEXT NOT NULL DEFAULT '', from_addr TEXT NOT NULL DEFAULT '',
            to_addrs TEXT NOT NULL DEFAULT '', cc_addrs TEXT NOT NULL DEFAULT '',
            date INTEGER NOT NULL DEFAULT 0, size INTEGER NOT NULL DEFAULT 0,
            seen INTEGER NOT NULL DEFAULT 0, flagged INTEGER NOT NULL DEFAULT 0,
            answered INTEGER NOT NULL DEFAULT 0, has_attach INTEGER NOT NULL DEFAULT 0,
            preview TEXT NOT NULL DEFAULT '', raw BLOB, UNIQUE(folder_id, uid));
         CREATE INDEX messages_folder_date ON messages(folder_id, date DESC);
         CREATE INDEX messages_folder_seen ON messages(folder_id, seen);
         INSERT INTO accounts VALUES (1, 'a@x.org', '', 'h', 993, 1, 'h', 465, 1, 1, 0);
         INSERT INTO folders VALUES (1, 1, 'INBOX', 'Inbox', 0, 'inbox', 7);
         INSERT INTO messages (id, account_id, folder_id, uid, raw) VALUES (1, 1, 1, 10, x'6869');
         INSERT INTO messages (id, account_id, folder_id, uid) VALUES (2, 1, 1, 11);
         PRAGMA user_version = 1;";

    #[test]
    fn migrates_v1_inline_bodies() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(V1_CACHE).unwrap();
        let db = Db::init(conn).unwrap();
        // Upgraded through v2 to the current schema in one go.
        assert_eq!(user_version(&db.conn).unwrap(), SCHEMA_VERSION);
        assert_eq!(db.folder(1).unwrap().unwrap().uidvalidity, 7);
        assert_eq!(db.fingerprint(1).unwrap(), None);
        assert_eq!(
            db.message(1).unwrap().unwrap().raw.as_deref(),
            Some(&b"hi"[..])
        );
        assert!(db.message(2).unwrap().unwrap().raw.is_none());
        assert_eq!(db.missing_bodies(1, 10, 1_000_000).unwrap(), [(2, 11, 0)]);
        // Deleting a message also deletes its body.
        db.delete_message(1).unwrap();
        let bodies: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM bodies", [], |r| r.get(0))
            .unwrap();
        assert_eq!(bodies, 0);
    }

    #[test]
    fn bodies_and_missing_list() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        db.insert_headers(acc, f, &[header(1, 1, "a"), header(2, 2, "b")])
            .unwrap();
        let missing = db.missing_bodies(f, 10, 1_000_000).unwrap();
        assert_eq!(missing.iter().map(|m| m.1).collect::<Vec<_>>(), [2, 1]);
        db.set_body(missing[0].0, b"raw", "preview", true).unwrap();
        assert_eq!(db.missing_bodies(f, 10, 1_000_000).unwrap().len(), 1);
        let m = db.message(missing[0].0).unwrap().unwrap();
        assert_eq!(m.raw.as_deref(), Some(&b"raw"[..]));
        let first = db.message_ids(&[f], "").unwrap()[0];
        assert_eq!(db.header(first).unwrap().unwrap().preview, "preview");
    }

    #[test]
    fn missing_bodies_only_looks_at_the_window() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        let headers: Vec<_> = (1..=400).map(|u| header(u, u as i64, "m")).collect();
        db.insert_headers(acc, f, &headers).unwrap();
        for (id, _, _) in db.missing_bodies(f, 300, 1_000_000).unwrap() {
            db.set_body(id, b"raw", "", false).unwrap();
        }
        // The 100 older ones are outside the window, so nothing is missing.
        assert!(db.missing_bodies(f, 300, 1_000_000).unwrap().is_empty());
        db.insert_headers(acc, f, &[header(401, 401, "new")])
            .unwrap();
        let missing = db.missing_bodies(f, 300, 1_000_000).unwrap();
        assert_eq!(missing.iter().map(|m| m.1).collect::<Vec<_>>(), [401]);
    }

    fn draft(account_id: i64) -> DraftRow {
        DraftRow {
            account_id,
            kind: 1,
            in_reply_to: "m1@x.org".into(),
            refs: "<m0@x.org> <m1@x.org>".into(),
            to: "Ana <ana@x.org>".into(),
            cc: "cy@x.org".into(),
            bcc: "dee@x.org".into(),
            subject: "Re: Plans".into(),
            body: "Sounds good.\n\n> Plans?".into(),
            message_id: "<d1@example.com>".into(),
            ..Default::default()
        }
    }

    #[test]
    fn migrates_v4_to_v5() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        db.insert_headers(acc, f, &[header(1, 1, "a")]).unwrap();
        let id = db.message_ids(&[f], "").unwrap()[0];
        db.set_body(id, b"raw", "preview", false).unwrap();
        db.allow_remote("news@shop.example").unwrap();
        db.conn
            .execute_batch(&format!(
                "{DROP_V12} {DROP_V11} {DROP_V10} {DROP_V9} {DROP_V8} {DROP_V7} {DROP_V5} PRAGMA user_version = 4;"
            ))
            .unwrap();

        let db = Db::init(db.conn).unwrap();
        assert_eq!(user_version(&db.conn).unwrap(), SCHEMA_VERSION);
        assert_eq!(db.count_drafts().unwrap(), 0);
        assert!(!db.header(id).unwrap().unwrap().has_draft);
        db.insert_draft(&DraftRow {
            reply_to_msg: Some(id),
            ..draft(acc)
        })
        .unwrap();
        assert!(db.header(id).unwrap().unwrap().has_draft);
        assert!(db.remote_allowed("news@shop.example").unwrap());
        assert_eq!(
            db.message(id).unwrap().unwrap().raw.as_deref(),
            Some(&b"raw"[..])
        );
    }

    #[test]
    fn drafts_round_trip_with_attachment_copies() {
        let tmp = TempDb::new("drafts");
        let db = Db::open(&tmp.0).unwrap();
        let acc = account(&db);
        let saved = draft(acc);
        let id = db.insert_draft(&saved).unwrap();
        let loaded = db.draft(id).unwrap().unwrap();
        assert_eq!(
            loaded,
            DraftRow {
                id,
                updated: loaded.updated,
                ..saved.clone()
            }
        );
        assert!(loaded.updated > 0);

        // Fields change with one update; the Message-ID stays.
        let edited = DraftRow {
            body: "Changed".into(),
            quoted_html: None,
            ..loaded
        };
        db.update_draft(&edited).unwrap();
        let again = Db::open(&tmp.0).unwrap().draft(id).unwrap().unwrap();
        assert_eq!(again.body, "Changed");
        assert_eq!(again.quoted_html, None);
        assert_eq!(again.message_id, "<d1@example.com>");

        let a = db
            .add_draft_file(id, "a.pdf", "application/pdf", b"%PDF")
            .unwrap();
        let b = db
            .add_draft_file(id, "b.txt", "text/plain", b"bee")
            .unwrap();
        assert_eq!(db.draft_files(id).unwrap(), [a.clone(), b.clone()]);
        assert_eq!(b.size, 3);
        let folder = drafts_dir(&tmp.0).join(id.to_string());
        assert_eq!(
            std::fs::read(folder.join(b.id.to_string())).unwrap(),
            b"bee"
        );
        assert_eq!(db.read_draft_file(id, &a, 100).unwrap(), b"%PDF");
        // A copy past the limit is refused, not read in full.
        assert!(db.read_draft_file(id, &a, 3).is_err());
        let header = db.draft_header(id).unwrap().unwrap();
        assert!(header.has_attach);
        assert_eq!(header.subject, "Re: Plans");

        db.remove_draft_file(id, a.id).unwrap();
        assert_eq!(db.draft_files(id).unwrap(), std::slice::from_ref(&b));
        assert!(!folder.join(a.id.to_string()).exists());

        assert_eq!(db.count_drafts().unwrap(), 1);
        assert_eq!(db.draft_ids("").unwrap(), [id]);
        assert_eq!(db.draft_ids("CHANGED").unwrap(), [id]);
        assert_eq!(db.draft_ids("ana@").unwrap(), [id]);
        assert!(db.draft_ids("nothing").unwrap().is_empty());

        // Deleting the draft removes its rows and its folder.
        db.delete_draft(id).unwrap();
        assert!(db.draft(id).unwrap().is_none());
        assert!(db.draft_files(id).unwrap().is_empty());
        assert!(!folder.exists());
        assert_eq!(db.count_drafts().unwrap(), 0);

        // A new draft that reuses the id starts without old copies.
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("1"), b"old").unwrap();
        assert_eq!(db.insert_draft(&saved).unwrap(), id);
        assert!(!folder.exists());

        // An in-memory cache has nowhere to keep copies.
        let mem = Db::in_memory().unwrap();
        let acc = account(&mem);
        let d = mem.insert_draft(&draft(acc)).unwrap();
        assert!(mem.add_draft_file(d, "x", "", b"x").is_err());
        assert!(mem.draft_files(d).unwrap().is_empty());
    }

    #[test]
    fn removing_an_account_removes_its_drafts_and_copies() {
        let tmp = TempDb::new("drafts-account");
        let db = Db::open(&tmp.0).unwrap();
        let acc = account(&db);
        let other = db
            .add_account(&NewAccount {
                imap_user: String::new(),
                smtp_user: String::new(),
                email: "other@example.com".into(),
                ..a_to_new(&db.accounts().unwrap()[0])
            })
            .unwrap();
        let mine = db.insert_draft(&draft(acc)).unwrap();
        let theirs = db.insert_draft(&draft(other)).unwrap();
        db.add_draft_file(mine, "a.txt", "text/plain", b"a")
            .unwrap();
        db.add_draft_file(theirs, "b.txt", "text/plain", b"b")
            .unwrap();
        let folder = |id: i64| drafts_dir(&tmp.0).join(id.to_string());
        assert!(folder(mine).exists());

        db.remove_account(acc).unwrap();
        assert!(db.draft(mine).unwrap().is_none());
        assert!(!folder(mine).exists());
        let left: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM draft_attachments", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 1);
        assert!(db.draft(theirs).unwrap().is_some());
        assert!(folder(theirs).exists());
    }

    #[test]
    fn a_reply_draft_forgets_a_message_that_leaves_the_cache() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        db.insert_headers(acc, f, &[header(1, 1, "a"), header(2, 2, "b")])
            .unwrap();
        let ids = db.message_ids(&[f], "").unwrap();
        let (newest, older) = (ids[0], ids[1]);
        let d = db
            .insert_draft(&DraftRow {
                reply_to_msg: Some(newest),
                ..draft(acc)
            })
            .unwrap();
        assert_eq!(db.draft_for_message(newest).unwrap(), Some(d));
        assert_eq!(db.draft_for_message(older).unwrap(), None);
        assert!(db.header(newest).unwrap().unwrap().has_draft);
        assert!(!db.header(older).unwrap().unwrap().has_draft);

        // The message goes (moved, expunged) and a new one takes its id.
        db.delete_message(newest).unwrap();
        assert_eq!(db.draft(d).unwrap().unwrap().reply_to_msg, None);
        db.insert_headers(acc, f, &[header(3, 3, "c")]).unwrap();
        let new = db.message_ids(&[f], "").unwrap()[0];
        assert_eq!(new, newest);
        assert!(!db.header(new).unwrap().unwrap().has_draft);
        // The draft itself stays.
        assert_eq!(db.count_drafts().unwrap(), 1);

        // Saving a reply to a message that is already gone still works (the
        // link is dropped), and so does saving a draft whose row vanished.
        let gone = DraftRow {
            reply_to_msg: Some(9999),
            ..draft(acc)
        };
        let g = db.insert_draft(&gone).unwrap();
        assert_eq!(db.draft(g).unwrap().unwrap().reply_to_msg, None);
        assert!(
            db.update_draft(&DraftRow {
                id: g,
                reply_to_msg: Some(9999),
                ..gone.clone()
            })
            .unwrap()
        );
        db.delete_draft(g).unwrap();
        assert!(!db.update_draft(&DraftRow { id: g, ..gone }).unwrap());
    }

    #[test]
    fn draft_count_and_reply_lookup_use_indexes() {
        let db = Db::in_memory().unwrap();
        let plan = |sql: &str| -> Vec<String> {
            db.conn
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .unwrap()
                .query_map([], |r| r.get(3))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        let count = plan("SELECT COUNT(*) FROM drafts WHERE state = 0");
        assert!(
            count.iter().any(|p| p.contains("drafts_state")),
            "{count:?}"
        );
        let reply = plan("SELECT 1 FROM drafts WHERE reply_to_msg = 7");
        assert!(
            reply.iter().any(|p| p.contains("drafts_reply")),
            "{reply:?}"
        );
    }

    #[test]
    fn migrates_v5_to_v6() {
        let db = Db::in_memory().unwrap();
        let acc = account(&db);
        let id = db.insert_draft(&draft(acc)).unwrap();
        db.conn
            .execute_batch(&format!(
                "{DROP_V12} {DROP_V11} {DROP_V10} {DROP_V9} {DROP_V8} {DROP_V7} ALTER TABLE drafts DROP COLUMN raw; ALTER TABLE drafts DROP COLUMN attempts;
                 ALTER TABLE drafts DROP COLUMN next_try; ALTER TABLE drafts DROP COLUMN last_error;
                 PRAGMA user_version = 5;"
            ))
            .unwrap();

        let db = Db::init(db.conn).unwrap();
        assert_eq!(user_version(&db.conn).unwrap(), SCHEMA_VERSION);
        // The draft stays a draft, and the outbox starts empty.
        assert_eq!(db.draft(id).unwrap().unwrap().subject, "Re: Plans");
        assert_eq!(
            (db.count_drafts().unwrap(), db.count_outbox().unwrap()),
            (1, 0)
        );
        assert!(db.outbox_send(id).unwrap());
        assert_eq!(
            (db.count_drafts().unwrap(), db.count_outbox().unwrap()),
            (0, 1)
        );
    }

    #[test]
    fn migrates_v6_to_v7() {
        let db = Db::in_memory().unwrap();
        let acc = account(&db);
        db.conn
            .execute_batch(&format!(
                "{DROP_V12} {DROP_V11} {DROP_V10} {DROP_V9} {DROP_V8} {DROP_V7} PRAGMA user_version = 6;"
            ))
            .unwrap();

        let db = Db::init(db.conn).unwrap();
        assert_eq!(user_version(&db.conn).unwrap(), SCHEMA_VERSION);
        let a = db.accounts().unwrap().remove(0);
        assert_eq!((a.id, a.signature.as_str()), (acc, ""));
        assert_eq!(db.settings().unwrap(), Settings::default());
        db.set_setting("undo_send_secs", "10").unwrap();
        assert_eq!(db.settings().unwrap().undo_send_secs, 10);
    }

    #[test]
    fn settings_default_when_missing_or_unreadable() {
        let db = Db::in_memory().unwrap();
        assert_eq!(db.settings().unwrap().undo_send_secs, 5);
        db.set_setting("undo_send_secs", "0").unwrap();
        assert_eq!(db.settings().unwrap().undo_send_secs, 0);
        db.set_setting("undo_send_secs", "lots").unwrap();
        assert_eq!(db.settings().unwrap().undo_send_secs, 5);
        db.set_setting("undo_send_secs", "86400").unwrap();
        assert_eq!(db.settings().unwrap().undo_send_secs, MAX_UNDO_SEND_SECS);
        // Unknown keys (from a newer version) are kept and ignored.
        db.set_setting("from_a_newer_version", "dark").unwrap();
        assert_eq!(db.settings().unwrap().undo_send_secs, MAX_UNDO_SEND_SECS);
        // The diagnostics log is off unless switched on.
        assert!(!db.settings().unwrap().diagnostics_log);
        db.set_setting("diagnostics_log", "1").unwrap();
        assert!(db.settings().unwrap().diagnostics_log);
        db.set_setting("diagnostics_log", "yes").unwrap();
        assert!(!db.settings().unwrap().diagnostics_log);
        // The language follows Windows until one is chosen; junk is ignored.
        use crate::i18n::LangSetting;
        assert_eq!(db.settings().unwrap().language, LangSetting::System);
        db.set_setting("language", "pt-PT").unwrap();
        assert_eq!(
            db.settings().unwrap().language,
            LangSetting::PortuguesePortugal
        );
        // Brazilian Portuguese is not offered: it reads as the default.
        db.set_setting("language", "pt-BR").unwrap();
        assert_eq!(db.settings().unwrap().language, LangSetting::System);
        db.set_setting("language", "en").unwrap();
        assert_eq!(db.settings().unwrap().language, LangSetting::English);
    }

    #[test]
    fn mark_read_delay_setting() {
        let db = Db::in_memory().unwrap();
        let read = || db.settings().unwrap().mark_read;
        assert_eq!(read(), MarkRead::After(Duration::from_secs(1)));
        for (value, want) in [
            ("0", MarkRead::Now),
            ("-1", MarkRead::Never),
            ("-5", MarkRead::Never),
            (" 250 ", MarkRead::After(Duration::from_millis(250))),
            ("999999999", MarkRead::After(Duration::from_secs(60))),
        ] {
            db.set_setting("mark_read_delay_ms", value).unwrap();
            assert_eq!(read(), want, "{value}");
        }
        // Unreadable: the default.
        db.set_setting("mark_read_delay_ms", "soon").unwrap();
        assert_eq!(read(), MarkRead::After(Duration::from_secs(1)));
    }

    #[test]
    fn update_account_keeps_folders_messages_and_drafts() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        db.insert_headers(acc, f, &[header(1, 100, "One"), header(2, 200, "Two")])
            .unwrap();
        let id = db.message_ids(&[f], "").unwrap()[0];
        db.set_body(id, b"Subject: Two\r\n\r\nHi", "Hi", false)
            .unwrap();
        db.insert_draft(&draft(acc)).unwrap();
        let counts = |db: &Db| -> Vec<i64> {
            ["folders", "messages", "bodies", "drafts"]
                .iter()
                .map(|t| {
                    db.conn
                        .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
                        .unwrap()
                })
                .collect()
        };
        let before = counts(&db);

        let mut a = db.accounts().unwrap().remove(0);
        a.name = "Me Too".into();
        a.imap = Server {
            host: "mail.example.net".into(),
            port: 143,
            security: Security::StartTls,
        };
        a.save_sent = false;
        a.signature = "Me
example.com"
            .into();
        // The address is the account's identity: never changed here.
        a.email = "other@example.com".into();
        assert!(db.update_account(&a).unwrap());

        let b = db.accounts().unwrap().remove(0);
        assert_eq!(b.email, "me@example.com");
        assert_eq!((b.name.as_str(), b.save_sent), ("Me Too", false));
        assert_eq!(b.imap, a.imap);
        assert_eq!(b.smtp.security, Security::StartTls);
        assert_eq!(
            b.signature,
            "Me
example.com"
        );
        assert_eq!(counts(&db), before);
        assert_eq!(
            db.message(id).unwrap().unwrap().raw.as_deref(),
            Some(&b"Subject: Two\r\n\r\nHi"[..])
        );
        // A removed account is not brought back.
        db.remove_account(acc).unwrap();
        assert!(!db.update_account(&a).unwrap());
        assert!(db.accounts().unwrap().is_empty());
    }

    /// Draft `id` moved through Send and its built message into the queue.
    fn queue(db: &Db, id: i64, raw: &[u8]) {
        assert!(db.outbox_send(id).unwrap());
        assert!(db.store_raw(id, raw, &AtomicBool::new(false)).unwrap());
        assert!(db.outbox_queue(id).unwrap());
    }

    #[test]
    fn outbox_rows_move_one_step_at_a_time() {
        let db = Db::in_memory().unwrap();
        let acc = account(&db);
        let id = db.insert_draft(&draft(acc)).unwrap();
        assert!(db.is_saved_draft(id).unwrap());

        // Not queued before the message is built.
        assert!(db.outbox_send(id).unwrap());
        assert!(!db.outbox_send(id).unwrap(), "already sent");
        assert!(!db.outbox_queue(id).unwrap());
        assert_eq!(db.outbox_pending().unwrap(), [(id, false)]);
        assert!(db.claim_outbox(id).unwrap().is_none());
        // Frozen: an autosave can't change it, and it leaves Drafts.
        assert!(!db.update_draft(&DraftRow { id, ..draft(acc) }).unwrap());
        assert!(
            !db.is_saved_draft(id).unwrap(),
            "not editable from a stale list"
        );
        assert!(db.draft_ids("").unwrap().is_empty());
        assert_eq!(db.outbox_ids("plans").unwrap(), [id]);

        assert!(db.store_raw(id, b"RAW", &AtomicBool::new(false)).unwrap());
        assert_eq!(db.outbox_pending().unwrap(), [(id, true)]);
        assert!(db.outbox_queue(id).unwrap());
        assert_eq!(db.outbox_due(0).unwrap(), [(id, acc)]);

        // Only one send gets it.
        let out = db.claim_outbox(id).unwrap().expect("queued");
        assert!(db.claim_outbox(id).unwrap().is_none());
        assert_eq!((out.raw.as_slice(), out.attempts), (&b"RAW"[..], 1));
        assert_eq!(
            (out.to.as_str(), out.bcc.as_str()),
            ("Ana <ana@x.org>", "dee@x.org")
        );
        // While it sends, nothing else may touch it.
        assert!(!db.outbox_unsend(id).unwrap());
        assert!(!db.discard_outbox(id).unwrap().0);
        assert!(!db.outbox_retry(id).unwrap());

        // A retry later: queued, due only at its time.
        db.outbox_result(id, Outbox::Queued, Some(1_000), "busy")
            .unwrap();
        assert!(db.outbox_due(999).unwrap().is_empty());
        assert_eq!(db.outbox_due(1_000).unwrap(), [(id, acc)]);
        assert_eq!(db.claim_outbox(id).unwrap().unwrap().attempts, 2);
        db.outbox_result(id, Outbox::Failed, None, "550 no")
            .unwrap();
        let e = db.outbox_entry(id).unwrap().unwrap();
        assert_eq!((e.state, e.last_error.as_str()), (Outbox::Failed, "550 no"));
        assert!(db.outbox_due(i64::MAX).unwrap().is_empty());

        // Retry queues it again from scratch; Edit makes it a draft.
        assert!(db.outbox_retry(id).unwrap());
        assert_eq!(db.claim_outbox(id).unwrap().unwrap().attempts, 1);
        db.outbox_result(id, Outbox::Unconfirmed, None, "may have")
            .unwrap();
        assert!(db.outbox_unsend(id).unwrap());
        assert_eq!(db.draft_ids("").unwrap(), [id]);
        assert!(db.outbox_entry(id).unwrap().is_none());
        assert_eq!(db.count_unsent(acc).unwrap(), 1);
        // Its built message went with it: a new Send builds again.
        assert!(db.outbox_send(id).unwrap());
        assert!(!db.outbox_queue(id).unwrap());
    }

    #[test]
    fn undo_beats_a_late_build() {
        let db = Db::in_memory().unwrap();
        let acc = account(&db);
        let id = db.insert_draft(&draft(acc)).unwrap();
        assert!(db.outbox_send(id).unwrap());
        // Undo: the flag first, then the row.
        let cancel = AtomicBool::new(false);
        cancel.store(true, Ordering::SeqCst);
        assert!(db.outbox_unsend(id).unwrap());
        assert!(!db.store_raw(id, b"late", &cancel).unwrap());
        // Sent again at once: the first build's result still can't land.
        assert!(db.outbox_send(id).unwrap());
        assert!(!db.store_raw(id, b"late", &cancel).unwrap());
        assert!(!db.outbox_queue(id).unwrap());
        // A build that stored first is cleared by the Undo.
        assert!(db.store_raw(id, b"new", &AtomicBool::new(false)).unwrap());
        assert!(db.outbox_unsend(id).unwrap());
        assert!(db.outbox_send(id).unwrap());
        assert!(!db.outbox_queue(id).unwrap(), "the old build was dropped");
    }

    #[test]
    fn a_send_cut_short_by_a_restart_is_unconfirmed() {
        let db = Db::in_memory().unwrap();
        let acc = account(&db);
        let sending = db.insert_draft(&draft(acc)).unwrap();
        let queued = db.insert_draft(&draft(acc)).unwrap();
        queue(&db, sending, b"A");
        queue(&db, queued, b"B");
        db.claim_outbox(sending).unwrap().unwrap();
        assert_eq!(db.recover_outbox("stopped").unwrap(), 1);
        let e = db.outbox_entry(sending).unwrap().unwrap();
        assert_eq!(
            (e.state, e.last_error.as_str()),
            (Outbox::Unconfirmed, "stopped")
        );
        assert!(db.claim_outbox(sending).unwrap().is_none());
        assert_eq!(db.outbox_due(0).unwrap(), [(queued, acc)]);
        assert_eq!(db.count_outbox().unwrap(), 2);
    }

    #[test]
    fn discarding_an_outbox_row_removes_its_copies() {
        let tmp = TempDb::new("outbox");
        let db = Db::open(&tmp.0).unwrap();
        let acc = account(&db);
        let id = db.insert_draft(&draft(acc)).unwrap();
        db.add_draft_file(id, "a.txt", "text/plain", b"a").unwrap();
        queue(&db, id, b"RAW");
        let folder = drafts_dir(&tmp.0).join(id.to_string());
        assert!(folder.exists());
        assert!(db.outbox_entry(id).unwrap().unwrap().has_attach);
        assert!(db.discard_outbox(id).unwrap().0);
        assert!(!folder.exists());
        assert_eq!(db.count_outbox().unwrap(), 0);
        // A worker's connection finds the same folder (a delivered send
        // deletes the row with it).
        let id = db.insert_draft(&draft(acc)).unwrap();
        db.add_draft_file(id, "b.txt", "text/plain", b"b").unwrap();
        let worker = Db::open_worker(&tmp.0).unwrap();
        worker.delete_draft(id).unwrap();
        assert!(!drafts_dir(&tmp.0).join(id.to_string()).exists());
    }

    /// The index row of `addr`: (name, weight, last_seen).
    fn indexed(db: &Db, addr: &str) -> Option<(String, i64, i64)> {
        db.conn
            .query_row(
                "SELECT name, weight, last_seen FROM addresses WHERE addr = ?1",
                [addr],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .unwrap()
    }

    fn suggested(db: &Db, account: i64, prefix: &str) -> Vec<String> {
        db.suggest_addresses(account, prefix, 6)
            .unwrap()
            .into_iter()
            .map(|r| r.addr)
            .collect()
    }

    /// The change count draft `id` of account `acc` is to be copied at.
    fn to_upload(db: &Db, acc: i64, id: i64) -> i64 {
        let found = db.drafts_to_upload(acc).unwrap();
        assert_eq!(found.iter().map(|d| d.0).collect::<Vec<_>>(), [id]);
        found[0].1
    }

    fn rev_of(db: &Db, id: i64) -> i64 {
        db.conn
            .query_row("SELECT rev FROM drafts WHERE id = ?1", [id], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn migrates_v8_to_v9() {
        let db = Db::in_memory().unwrap();
        let acc = account(&db);
        let drafts = db
            .upsert_folder(acc, "Drafts", "Drafts", 0, Role::Drafts)
            .unwrap();
        let id = db.insert_draft(&draft(acc)).unwrap();
        db.conn
            .execute_batch(&format!(
                "{DROP_V12} {DROP_V11} {DROP_V10} {DROP_V9} PRAGMA user_version = 8;"
            ))
            .unwrap();

        let db = Db::init(db.conn).unwrap();
        assert_eq!(user_version(&db.conn).unwrap(), SCHEMA_VERSION);
        // A draft saved before has never been copied to the server.
        assert_eq!(to_upload(&db, acc, id), rev_of(&db, id));
        let copy = ServerCopy {
            account_id: acc,
            folder_id: drafts,
            uidvalidity: 7,
            uid: 3,
        };
        let rev = to_upload(&db, acc, id);
        assert_eq!(
            db.record_server_copy(id, rev, Some(&copy)).unwrap(),
            Recorded::Replaced(None)
        );
        assert!(db.drafts_to_upload(acc).unwrap().is_empty());
        assert_eq!(db.delete_draft(id).unwrap(), Some(copy));
    }

    #[test]
    fn migrates_v9_to_v10() {
        let db = Db::in_memory().unwrap();
        let acc = account(&db);
        let id = db.insert_draft(&draft(acc)).unwrap();
        db.conn
            .execute_batch(&format!(
                "{DROP_V12} {DROP_V11} {DROP_V10} PRAGMA user_version = 9;"
            ))
            .unwrap();

        let db = Db::init(db.conn).unwrap();
        assert_eq!(user_version(&db.conn).unwrap(), SCHEMA_VERSION);
        assert!(db.draft(id).unwrap().is_some());
        assert_eq!(db.rsvp(acc, "u@x", "").unwrap(), None);
        db.record_rsvp(acc, "u@x", "", 2, 1).unwrap();
        assert_eq!(db.rsvp(acc, "u@x", "").unwrap(), Some((2, 1)));
    }

    #[test]
    fn migrates_v10_to_v11() {
        let db = Db::in_memory().unwrap();
        let acc = account(&db);
        db.set_setting("undo_send_secs", "10").unwrap();
        db.conn
            .execute_batch(&format!("{DROP_V12} {DROP_V11} PRAGMA user_version = 10;"))
            .unwrap();

        let db = Db::init(db.conn).unwrap();
        assert_eq!(user_version(&db.conn).unwrap(), SCHEMA_VERSION);
        // Accounts start switched on.
        let a = db.accounts().unwrap().remove(0);
        assert_eq!((a.id, a.enabled), (acc, true));
        assert_eq!(db.settings().unwrap().undo_send_secs, 10);
        db.set_account_enabled(acc, false).unwrap();
        assert!(!db.accounts().unwrap()[0].enabled);
    }

    #[test]
    fn migrates_v11_to_v12() {
        let db = Db::in_memory().unwrap();
        let acc = account(&db);
        db.conn
            .execute_batch(&format!("{DROP_V12} PRAGMA user_version = 11;"))
            .unwrap();

        let db = Db::init(db.conn).unwrap();
        assert_eq!(user_version(&db.conn).unwrap(), SCHEMA_VERSION);
        assert_eq!(SCHEMA_VERSION, 12);
        let a = db.accounts().unwrap().remove(0);
        assert_eq!(
            (a.id, a.email.as_str(), a.name.as_str()),
            (acc, "me@example.com", "Me")
        );
        assert_eq!(a.imap.host, "imap.example.com");
        assert_eq!((a.imap_user.as_str(), a.smtp_user.as_str()), ("", ""));
        assert!(a.imap_pin.is_none() && a.smtp_pin.is_none());
        assert_eq!(a.imap_login(), "me@example.com");
        assert_eq!(a.smtp_login(), "me@example.com");
        // The folder role column starts as "follow the detected role".
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        let over: String = db
            .conn
            .query_row(
                "SELECT role_override FROM folders WHERE id = ?1",
                [f],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(over, "");
        // A second init changes nothing.
        let db = Db::init(db.conn).unwrap();
        assert_eq!(user_version(&db.conn).unwrap(), 12);
        assert_eq!(db.accounts().unwrap().len(), 1);
    }

    #[test]
    fn logins_and_pins_round_trip() {
        let db = Db::in_memory().unwrap();
        let acc = account(&db);
        let a = db.accounts().unwrap().remove(0);
        db.update_account(&Account {
            imap_user: "alice".into(),
            smtp_user: "alice@smtp".into(),
            imap_pin: Some(vec![1, 2, 3]),
            ..a
        })
        .unwrap();
        let a = db.accounts().unwrap().remove(0);
        assert_eq!(a.id, acc);
        assert_eq!((a.imap_login(), a.smtp_login()), ("alice", "alice@smtp"));
        assert_eq!(a.imap_pin.as_deref(), Some(&[1u8, 2, 3][..]));
        assert!(a.smtp_pin.is_none());
        // Blanking a login goes back to the address.
        db.update_account(&Account {
            imap_user: String::new(),
            ..a
        })
        .unwrap();
        assert_eq!(db.accounts().unwrap()[0].imap_login(), "me@example.com");
    }

    #[test]
    fn accounts_reorder_and_switch_off() {
        let db = Db::in_memory().unwrap();
        let add = |email: &str| {
            db.add_account(&NewAccount {
                imap_user: String::new(),
                smtp_user: String::new(),
                email: email.into(),
                name: String::new(),
                imap: Server {
                    host: "imap.example.com".into(),
                    port: 993,
                    security: Security::Tls,
                },
                smtp: Server {
                    host: "smtp.example.com".into(),
                    port: 465,
                    security: Security::Tls,
                },
                save_sent: true,
            })
            .unwrap()
        };
        let (a, b, c) = (add("a@x.org"), add("b@x.org"), add("c@x.org"));
        // An older cache may hold equal positions: they still swap.
        db.conn
            .execute("UPDATE accounts SET position = 0", [])
            .unwrap();
        let order = || -> Vec<i64> { db.accounts().unwrap().iter().map(|x| x.id).collect() };
        assert_eq!(order(), [a, b, c]);
        assert!(db.move_account(c, true).unwrap());
        assert_eq!(order(), [a, c, b]);
        assert!(db.move_account(a, false).unwrap());
        assert_eq!(order(), [c, a, b]);
        // Nothing past either end, nor for an unknown account.
        assert!(!db.move_account(c, true).unwrap());
        assert!(!db.move_account(b, false).unwrap());
        assert!(!db.move_account(99, true).unwrap());
        assert_eq!(order(), [c, a, b]);

        db.set_account_enabled(a, false).unwrap();
        let enabled: Vec<bool> = db.accounts().unwrap().iter().map(|x| x.enabled).collect();
        assert_eq!(enabled, [true, false, true]);
    }

    #[test]
    fn redownload_keeps_the_account_drafts_and_settings() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let inbox = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        let drafts = db
            .upsert_folder(acc, "Drafts", "Drafts", 0, Role::Drafts)
            .unwrap();
        db.set_uidvalidity(inbox, 7).unwrap();
        db.insert_headers(acc, inbox, &[header(1, 100, "One"), header(2, 200, "Two")])
            .unwrap();
        let fp = Fp {
            uidnext: 3,
            messages: 2,
            hms: 9,
        };
        db.finish_folder(inbox, fp).unwrap();
        let id = db.message_ids(&[inbox], "").unwrap()[0];
        db.set_body(id, b"Subject: Two\r\n\r\nHi", "Hi", false)
            .unwrap();
        db.rebuild_addresses(acc).unwrap();
        db.backfill_addresses(acc, 100).unwrap();
        let mut d = draft(acc);
        d.reply_to_msg = Some(id);
        let draft_id = db.insert_draft(&d).unwrap();
        let copy = ServerCopy {
            account_id: acc,
            folder_id: drafts,
            uidvalidity: 5,
            uid: 4,
        };
        let rev = db.drafts_to_upload(acc).unwrap()[0].1;
        db.record_server_copy(draft_id, rev, Some(&copy)).unwrap();
        db.record_rsvp(acc, "u@x", "", 0, 1).unwrap();
        db.set_setting("undo_send_secs", "10").unwrap();
        db.update_account(&Account {
            signature: "Me".into(),
            ..db.accounts().unwrap().remove(0)
        })
        .unwrap();
        let count =
            |db: &Db, sql: &str| -> i64 { db.conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert!(count(&db, "SELECT COUNT(*) FROM addresses") > 0);

        db.redownload(acc).unwrap();
        // The downloaded mail and what was derived from it are gone...
        assert!(db.message_ids(&[inbox], "").unwrap().is_empty());
        assert_eq!(count(&db, "SELECT COUNT(*) FROM bodies"), 0);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM addresses"), 0);
        assert_eq!(db.fingerprint(inbox).unwrap(), None);
        // ...the account, its folders, drafts (with their server copy),
        // answers and the settings stay.
        let a = db.accounts().unwrap().remove(0);
        assert_eq!((a.id, a.signature.as_str(), a.enabled), (acc, "Me", true));
        assert_eq!(db.folders().unwrap().len(), 2);
        assert_eq!(db.folder(inbox).unwrap().unwrap().uidvalidity, 7);
        let kept = db.draft(draft_id).unwrap().unwrap();
        assert_eq!((kept.subject, kept.reply_to_msg), (d.subject, None));
        assert!(db.drafts_to_upload(acc).unwrap().is_empty());
        assert_eq!(db.rsvp(acc, "u@x", "").unwrap(), Some((0, 1)));
        assert_eq!(db.settings().unwrap().undo_send_secs, 10);
        assert_eq!(db.delete_draft(draft_id).unwrap(), Some(copy));
        // The next sync stores the headers again.
        db.insert_headers(acc, inbox, &[header(1, 100, "One")])
            .unwrap();
        assert_eq!(db.message_ids(&[inbox], "").unwrap().len(), 1);
    }

    #[test]
    fn general_settings() {
        let db = Db::in_memory().unwrap();
        let s = db.settings().unwrap();
        assert_eq!(
            (s.theme, s.check_interval_secs, s.flash_taskbar, s.window),
            (ThemeChoice::System, 60, false, None)
        );
        for t in [ThemeChoice::Dark, ThemeChoice::Light, ThemeChoice::System] {
            db.set_setting("theme", t.key()).unwrap();
            assert_eq!(db.settings().unwrap().theme, t);
        }
        db.set_setting("theme", "purple").unwrap();
        assert_eq!(db.settings().unwrap().theme, ThemeChoice::System);
        db.set_setting("check_interval_secs", "300").unwrap();
        assert_eq!(db.settings().unwrap().check_interval_secs, 300);
        // Only the offered intervals.
        db.set_setting("check_interval_secs", "1").unwrap();
        assert_eq!(db.settings().unwrap().check_interval_secs, 60);
        db.set_setting("flash_taskbar", "1").unwrap();
        assert!(db.settings().unwrap().flash_taskbar);
        // The onboarding's: the tips seen, and the version What's new told.
        assert_eq!(db.settings().unwrap().tips_seen, "");
        assert_eq!(db.settings().unwrap().whats_new_version, None);
        db.set_setting("tips_seen", "search,help").unwrap();
        db.set_setting("whats_new_version", "0.2.0").unwrap();
        let s = db.settings().unwrap();
        assert_eq!(s.tips_seen, "search,help");
        assert_eq!(s.whats_new_version.as_deref(), Some("0.2.0"));
        let g = Geometry {
            x: -1900,
            y: 20,
            w: 1280,
            h: 820,
            maximized: true,
        };
        db.set_setting("window", &g.to_setting()).unwrap();
        assert_eq!(db.settings().unwrap().window, Some(g));
        for bad in ["", "1 2 3 4", "1 2 0 4 0", "1 2 3 4 0 6", "a b c d e"] {
            assert_eq!(Geometry::from_setting(bad), None, "{bad}");
        }
    }

    #[test]
    fn invite_answers_and_replies() {
        let db = Db::in_memory().unwrap();
        let acc = account(&db);
        // One answer per event and occurrence; a new answer replaces it.
        db.record_rsvp(acc, "u@x", "", 0, 1).unwrap();
        db.record_rsvp(acc, "u@x", ":20261008T100000Z", 0, 3)
            .unwrap();
        db.record_rsvp(acc, "u@x", "", 1, 2).unwrap();
        assert_eq!(db.rsvp(acc, "u@x", "").unwrap(), Some((1, 2)));
        assert_eq!(
            db.rsvp(acc, "u@x", ":20261008T100000Z").unwrap(),
            Some((0, 3))
        );
        assert_eq!(db.rsvp(acc + 1, "u@x", "").unwrap(), None);
        // A reply is queued at once, built, and never goes back to Drafts.
        let row = db
            .queue_reply(
                &DraftRow {
                    account_id: acc,
                    to: "jane@x.org".into(),
                    subject: "Accepted: Plan".into(),
                    message_id: "<r1@example.com>".into(),
                    ..Default::default()
                },
                b"raw",
            )
            .unwrap();
        assert_eq!(db.outbox_due(0).unwrap(), [(row, acc)]);
        assert_eq!(db.count_drafts().unwrap(), 0);
        let e = db.outbox_entry(row).unwrap().unwrap();
        assert!(e.invite_reply && e.state == Outbox::Queued);
        assert!(!db.outbox_unsend(row).unwrap());
        assert_eq!(db.claim_outbox(row).unwrap().unwrap().raw, b"raw");
        // The answers go with their account.
        db.remove_account(acc).unwrap();
        assert_eq!(db.rsvp(acc, "u@x", "").unwrap(), None);
    }

    #[test]
    fn draft_changes_wait_for_their_server_copy() {
        let tmp = TempDb::new("server-copy");
        let db = Db::open(&tmp.0).unwrap();
        let acc = account(&db);
        let drafts = db
            .upsert_folder(acc, "Drafts", "Drafts", 0, Role::Drafts)
            .unwrap();
        let id = db.insert_draft(&draft(acc)).unwrap();
        let rev = to_upload(&db, acc, id);
        // Edited while its copy uploaded: still to upload after recording.
        db.update_draft(&DraftRow { id, ..draft(acc) }).unwrap();
        let copy = |uid| ServerCopy {
            account_id: acc,
            folder_id: drafts,
            uidvalidity: 7,
            uid,
        };
        db.record_server_copy(id, rev, Some(&copy(3))).unwrap();
        assert_eq!(to_upload(&db, acc, id), rev_of(&db, id));
        let rev = to_upload(&db, acc, id);
        assert_eq!(
            db.record_server_copy(id, rev, Some(&copy(4))).unwrap(),
            Recorded::Replaced(Some(copy(3)))
        );
        assert!(db.drafts_to_upload(acc).unwrap().is_empty());
        // Attachments are changes too.
        let file = db.add_draft_file(id, "a.txt", "text/plain", b"x").unwrap();
        assert_eq!(to_upload(&db, acc, id), rev_of(&db, id));
        let rev = to_upload(&db, acc, id);
        db.record_server_copy(id, rev, Some(&copy(5))).unwrap();
        db.remove_draft_file(id, file.id).unwrap();
        assert_eq!(to_upload(&db, acc, id), rev_of(&db, id));
        // Another account's worker never takes it; the outbox is no draft.
        assert!(db.drafts_to_upload(acc + 1).unwrap().is_empty());
        assert!(db.outbox_send(id).unwrap());
        assert!(db.drafts_to_upload(acc).unwrap().is_empty());
        assert_eq!(
            db.record_server_copy(id, rev, Some(&copy(6))).unwrap(),
            Recorded::NotADraft
        );
        // A vanished folder takes the copy with it.
        db.retain_folders(acc, &[]).unwrap();
        assert_eq!(db.delete_draft(id).unwrap(), None);
    }

    #[test]
    fn migrates_v7_to_v8() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let inbox = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        db.insert_headers(acc, inbox, &[header(1, 100, "a"), header(2, 200, "b")])
            .unwrap();
        db.conn
            .execute_batch(&format!(
                "{DROP_V12} {DROP_V11} {DROP_V10} {DROP_V9} {DROP_V8} PRAGMA user_version = 7;"
            ))
            .unwrap();

        let mut db = Db::init(db.conn).unwrap();
        assert_eq!(user_version(&db.conn).unwrap(), SCHEMA_VERSION);
        // The cached mail waits for the backfill; new mail is indexed at once.
        assert!(suggested(&db, acc, "jane").is_empty());
        let mut later = header(3, 300, "c");
        later.from_addr = "bob@example.com".into();
        db.insert_headers(acc, inbox, &[later]).unwrap();
        assert_eq!(suggested(&db, acc, "bob"), ["bob@example.com"]);
        while db.backfill_addresses(acc, BACKFILL_CHUNK).unwrap() {}
        assert_eq!(
            indexed(&db, "jane@example.com"),
            Some(("Jane".into(), 2, 200))
        );
        assert_eq!(indexed(&db, "bob@example.com").unwrap().1, 1);
        // Removing the account removes its index.
        db.remove_account(acc).unwrap();
        let left: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM addresses", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn sent_mail_weighs_more_and_junk_and_self_are_left_out() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let inbox = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        let sent = db
            .upsert_folder(acc, "Sent", "Sent", 0, Role::Sent)
            .unwrap();
        let junk = db
            .upsert_folder(acc, "Junk", "Junk", 0, Role::Junk)
            .unwrap();
        db.insert_headers(acc, inbox, &[header(1, 100, "hi")])
            .unwrap();
        let out = NewHeader {
            uid: 1,
            from_name: "Me".into(),
            from_addr: "Me@Example.com".into(),
            to: "Ana Silva <Ana@X.pt>, \"Doe, Jane\" <jane@example.com>".into(),
            cc: "carl@x.pt; not an address".into(),
            date: 300,
            ..Default::default()
        };
        db.insert_headers(acc, sent, std::slice::from_ref(&out))
            .unwrap();
        let mut spam = header(1, 400, "win");
        spam.from_addr = "spam@x.pt".into();
        db.insert_headers(acc, junk, &[spam]).unwrap();

        assert_eq!(indexed(&db, "ana@x.pt"), Some(("Ana Silva".into(), 3, 300)));
        assert_eq!(indexed(&db, "carl@x.pt"), Some((String::new(), 3, 300)));
        // An incoming sender (+1) the user also wrote to (+3); the newest
        // name wins.
        assert_eq!(
            indexed(&db, "jane@example.com"),
            Some(("Doe, Jane".into(), 4, 300))
        );
        assert_eq!(indexed(&db, "me@example.com"), None);
        assert_eq!(indexed(&db, "spam@x.pt"), None);
        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM addresses", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3);
        // A header the cache already holds counts once.
        db.insert_headers(acc, sent, &[out]).unwrap();
        assert_eq!(indexed(&db, "ana@x.pt").unwrap().1, 3);
        // An older message keeps the newer name.
        let mut old = header(2, 50, "old");
        old.from_name = "J. Doe".into();
        db.insert_headers(acc, inbox, &[old]).unwrap();
        assert_eq!(
            indexed(&db, "jane@example.com"),
            Some(("Doe, Jane".into(), 5, 300))
        );
    }

    #[test]
    fn address_backfill_is_chunked_and_idempotent() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let inbox = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        let headers: Vec<NewHeader> = (1..=5)
            .map(|uid| {
                let mut h = header(uid, i64::from(uid) * 100, "x");
                h.from_addr = format!("p{}@x.pt", uid % 3);
                h
            })
            .collect();
        db.insert_headers(acc, inbox, &headers).unwrap();
        let live: Vec<_> = (0..3)
            .map(|i| indexed(&db, &format!("p{i}@x.pt")))
            .collect();
        // As a cache from before the index: nothing indexed, all to backfill.
        db.rebuild_addresses(acc).unwrap();
        assert_eq!(indexed(&db, "p0@x.pt"), None);

        assert!(db.backfill_addresses(acc, 2).unwrap());
        // The newest two first: uids 5 (p2) and 4 (p1).
        assert_eq!(indexed(&db, "p2@x.pt").unwrap().1, 1);
        assert_eq!(indexed(&db, "p1@x.pt").unwrap().1, 1);
        assert_eq!(indexed(&db, "p0@x.pt"), None);
        assert!(db.backfill_addresses(acc, 2).unwrap());
        assert!(!db.backfill_addresses(acc, 2).unwrap());
        let filled: Vec<_> = (0..3)
            .map(|i| indexed(&db, &format!("p{i}@x.pt")))
            .collect();
        assert_eq!(filled, live);
        // Done: running it again changes nothing.
        assert!(!db.backfill_addresses(acc, 2).unwrap());
        assert!(!db.backfill_addresses(acc, BACKFILL_CHUNK).unwrap());
        let again: Vec<_> = (0..3)
            .map(|i| indexed(&db, &format!("p{i}@x.pt")))
            .collect();
        assert_eq!(again, live);
        // A new account starts with nothing to backfill.
        assert!(!db.backfill_addresses(acc + 1, 2).unwrap());
    }

    #[test]
    fn a_name_with_control_characters_is_not_indexed() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let inbox = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        let h = NewHeader {
            uid: 1,
            from_name: "Eve
Bcc: x"
                .into(),
            from_addr: "eve@x.pt".into(),
            date: 5,
            ..Default::default()
        };
        db.insert_headers(acc, inbox, &[h]).unwrap();
        let found = db.suggest_addresses(acc, "eve", 6).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "");
    }

    #[test]
    fn suggestions_match_address_and_accented_name_prefixes() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let sent = db
            .upsert_folder(acc, "Sent", "Sent", 0, Role::Sent)
            .unwrap();
        let out = NewHeader {
            uid: 1,
            from_addr: "me@example.com".into(),
            to: "João Silva <joao@x.pt>, Joana <jo_ana@x.pt>, Álvaro <alvaro@x.pt>".into(),
            date: 100,
            ..Default::default()
        };
        let inbox = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        db.insert_headers(acc, sent, &[out]).unwrap();
        // Two more messages from Joana: she ranks first.
        let from_joana = |uid| NewHeader {
            uid,
            from_name: "Joana".into(),
            from_addr: "jo_ana@x.pt".into(),
            date: 50,
            ..Default::default()
        };
        db.insert_headers(acc, inbox, &[from_joana(1), from_joana(2)])
            .unwrap();

        assert_eq!(suggested(&db, acc, "jo"), ["jo_ana@x.pt", "joao@x.pt"]);
        assert_eq!(suggested(&db, acc, " JOÃ"), ["joao@x.pt"]);
        assert_eq!(suggested(&db, acc, "joão s"), ["joao@x.pt"]);
        // Prefixes only: a later word of the name is no match.
        assert!(suggested(&db, acc, "silva").is_empty());
        assert_eq!(suggested(&db, acc, "joao@x"), ["joao@x.pt"]);
        assert_eq!(suggested(&db, acc, "álv"), ["alvaro@x.pt"]);
        // Nothing in a prefix is a wildcard.
        assert_eq!(suggested(&db, acc, "jo_"), ["jo_ana@x.pt"]);
        assert!(suggested(&db, acc, "%").is_empty());
        assert!(suggested(&db, acc, "").is_empty());
        // Only the account's own index.
        assert!(suggested(&db, acc + 1, "jo").is_empty());
        let top = db.suggest_addresses(acc, "jo", 1).unwrap();
        assert_eq!(
            top,
            [Recipient {
                name: "Joana".into(),
                addr: "jo_ana@x.pt".into()
            }]
        );
    }
}
