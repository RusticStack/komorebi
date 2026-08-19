//! GPUI mail shell.
//!
//! A four-column mail client: navigation sidebar, message list, conversation
//! reader (with inline composer), and a contextual Komorebi AI panel. The
//! shell is built on gpui-component primitives: a custom [`TitleBar`],
//! [`resizable`] panels, icons, avatars, badges, tooltips, and a custom
//! Japandi palette (warm charcoal surfaces, sand/beige accents). Blocking
//! mail/AI work runs off the UI thread; the [`App`] entity owns all local
//! view state.

use crate::ai::{self, Ai, AiConfig, Category};
use crate::db::{Account, Attachment, Database, Folder, MessageDetail, MessageRow, Reminder};
use crate::mail;
use crate::mail::discovery::DiscoveredSettings;
use crate::mail::types::{ProviderKind, Security, ServerSettings};
use gpui::prelude::*;
use gpui::{
    AnyElement, ClipboardItem, Context, Entity, ExternalPaths, FocusHandle, Focusable, Hsla,
    KeyBinding, Pixels, Render, SharedString, StatefulInteractiveElement, Subscription, Window,
    actions, div, px, uniform_list,
};
use gpui_component::avatar::Avatar;
use gpui_component::badge::Badge;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::label::Label;
use gpui_component::menu::{ContextMenuExt, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_component::resizable::{h_resizable, resizable_panel};
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _, Theme,
    ThemeMode, TitleBar, h_flex, v_flex,
};
use std::path::{Path, PathBuf};

const CONTEXT: &str = "Komorebi";
/// Sentinel ids for the virtual views in the sidebar (not real DB folders).
const PRIORITY: &str = "priority";
const SNOOZED: &str = "snoozed";
const LABEL_PREFIX: &str = "label:";
const VIEW_UNREAD: &str = "view:unread";
const VIEW_ATTACHMENTS: &str = "view:attachments";
const VIEW_MENTIONS: &str = "view:mentions";

/// Sidebar labels and their accent color (Japandi muted palette).
const LABELS: &[(&str, u32)] = &[
    ("Work", 0x6b9bd6),
    ("Design", 0xa58bd6),
    ("Personal", 0xa8c256),
    ("Finance", 0xd68b4e),
    ("Travel", 0xd66b5e),
];

actions!(
    komorebi,
    [
        NewMessage,
        FocusSearch,
        ToggleSidebar,
        ToggleAiPanel,
        NextMessage,
        PrevMessage,
        ArchiveMessage,
        StarMessage,
        ReplyMessage,
        ForwardMessage,
        ToggleTheme,
    ]
);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel {
    Mail,
    Compose,
    Accounts,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReplyMode {
    Reply,
    Note,
}

/// Connection security selection for manual account setup.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SetupSecurity {
    Tls,
    StartTls,
}

impl SetupSecurity {
    fn as_str(&self) -> &'static str {
        match self {
            SetupSecurity::Tls => "tls",
            SetupSecurity::StartTls => "starttls",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            SetupSecurity::Tls => "TLS",
            SetupSecurity::StartTls => "STARTTLS",
        }
    }

    fn security(&self) -> Security {
        match self {
            SetupSecurity::Tls => Security::Tls,
            SetupSecurity::StartTls => Security::StartTls,
        }
    }
}

#[derive(Clone, Copy)]
enum AiKind {
    Summarize,
    DraftReply,
    Polish,
    Triage,
    FollowUp,
}

impl AiKind {
    fn as_str(&self) -> &'static str {
        match self {
            AiKind::Summarize => "summarize",
            AiKind::DraftReply => "draft_reply",
            AiKind::Polish => "polish",
            AiKind::Triage => "triage",
            AiKind::FollowUp => "follow_up",
        }
    }
}

// ---------------------------------------------------------------------------
// Drag & drop payloads
// ---------------------------------------------------------------------------

/// Internal drag payload for moving a message onto a folder.
#[derive(Clone)]
struct DragMessage {
    message_id: String,
}

struct DragMessagePreview {
    message_id: String,
}

impl Render for DragMessagePreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = &self.message_id;
        div()
            .px_3()
            .py_1p5()
            .rounded_md()
            .shadow_md()
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .border_1()
            .border_color(cx.theme().border)
            .child("Move message")
    }
}

#[derive(Clone)]
struct PendingAttachment {
    name: String,
    size_bytes: i64,
    data: Vec<u8>,
}

/// Enough to undo the last snooze/follow-up action: restore the previous
/// reminder (if any) or clear the one we just scheduled.
struct UndoReminder {
    message_id: String,
    previous: Option<Reminder>,
}

/// Quick snooze choices: label + seconds from now.
const SNOOZE_OPTIONS: &[(&str, i64)] = &[
    ("Later today", 4 * 3600),
    ("Tomorrow", 24 * 3600),
    ("In 3 days", 3 * 24 * 3600),
    ("Next week", 7 * 24 * 3600),
];

/// An attachment written to disk for in-app image preview.
struct AttachmentPreview {
    name: String,
    path: PathBuf,
}

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

fn komorebi_hex(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}

/// Apply the Komorebi Japandi palette (warm charcoal + sand/beige accents).
fn apply_komorebi_theme(window: &mut Window, cx: &mut gpui::App) {
    let theme = Theme::global_mut(cx);
    if theme.mode.is_dark() {
        theme.colors.background = komorebi_hex(0x1b1816);
        theme.colors.foreground = komorebi_hex(0xefe9dc);
        theme.colors.muted = komorebi_hex(0x262220);
        theme.colors.muted_foreground = komorebi_hex(0x9a9284);
        theme.colors.border = komorebi_hex(0x2e2a26);
        theme.colors.primary = komorebi_hex(0xe6d9b8);
        theme.colors.primary_foreground = komorebi_hex(0x1f1a14);
        theme.colors.primary_hover = komorebi_hex(0xefe4c8);
        theme.colors.primary_active = komorebi_hex(0xdccda6);
        theme.colors.accent = komorebi_hex(0xe6d9b8);
        theme.colors.accent_foreground = komorebi_hex(0x1f1a14);
        theme.colors.link = komorebi_hex(0xc7b78f);
        theme.colors.sidebar = komorebi_hex(0x161311);
        theme.colors.sidebar_foreground = komorebi_hex(0xc9c2b4);
        theme.colors.sidebar_border = komorebi_hex(0x2e2a26);
        theme.colors.sidebar_accent = komorebi_hex(0x2a2520);
        theme.colors.sidebar_accent_foreground = komorebi_hex(0xf1ebdd);
        theme.colors.sidebar_primary = komorebi_hex(0xe6d9b8);
        theme.colors.sidebar_primary_foreground = komorebi_hex(0x1f1a14);
        theme.colors.list = komorebi_hex(0x1b1816);
        theme.colors.list_active = komorebi_hex(0x26221e);
        theme.colors.list_hover = komorebi_hex(0x221e1a);
        theme.colors.popover = komorebi_hex(0x26221e);
        theme.colors.popover_foreground = komorebi_hex(0xefe9dc);
        theme.colors.input = komorebi_hex(0x1e1a17);
        theme.colors.title_bar = komorebi_hex(0x161311);
        theme.colors.title_bar_border = komorebi_hex(0x2e2a26);
        theme.colors.window_border = komorebi_hex(0x2e2a26);
        theme.colors.danger = komorebi_hex(0xc9684e);
        theme.colors.warning = komorebi_hex(0xc99a4a);
        theme.colors.success = komorebi_hex(0x7e9a6b);
        theme.colors.info = komorebi_hex(0x6b7fa0);
        theme.colors.drop_target = komorebi_hex(0x33291f);
    } else {
        theme.colors.background = komorebi_hex(0xf6f1e7);
        theme.colors.foreground = komorebi_hex(0x2c2620);
        theme.colors.muted = komorebi_hex(0xece4d4);
        theme.colors.muted_foreground = komorebi_hex(0x7d756a);
        theme.colors.border = komorebi_hex(0xddd3c2);
        theme.colors.primary = komorebi_hex(0xb9a67e);
        theme.colors.primary_foreground = komorebi_hex(0x241f18);
        theme.colors.primary_hover = komorebi_hex(0xae9a70);
        theme.colors.primary_active = komorebi_hex(0xa28d61);
        theme.colors.accent = komorebi_hex(0xb9a67e);
        theme.colors.accent_foreground = komorebi_hex(0x241f18);
        theme.colors.link = komorebi_hex(0x8d7a52);
        theme.colors.sidebar = komorebi_hex(0xefe8da);
        theme.colors.sidebar_foreground = komorebi_hex(0x4a4238);
        theme.colors.sidebar_border = komorebi_hex(0xddd3c2);
        theme.colors.sidebar_accent = komorebi_hex(0xe2d8c2);
        theme.colors.sidebar_accent_foreground = komorebi_hex(0x2c2620);
        theme.colors.sidebar_primary = komorebi_hex(0xb9a67e);
        theme.colors.sidebar_primary_foreground = komorebi_hex(0x241f18);
        theme.colors.list = komorebi_hex(0xf6f1e7);
        theme.colors.list_active = komorebi_hex(0xece3d1);
        theme.colors.list_hover = komorebi_hex(0xf0e9db);
        theme.colors.popover = komorebi_hex(0xfbf7ee);
        theme.colors.popover_foreground = komorebi_hex(0x2c2620);
        theme.colors.input = komorebi_hex(0xfbf7ee);
        theme.colors.title_bar = komorebi_hex(0xefe8da);
        theme.colors.title_bar_border = komorebi_hex(0xddd3c2);
        theme.colors.window_border = komorebi_hex(0xddd3c2);
        theme.colors.danger = komorebi_hex(0xb2533c);
        theme.colors.warning = komorebi_hex(0xa87f2e);
        theme.colors.success = komorebi_hex(0x5f7d4e);
        theme.colors.info = komorebi_hex(0x4f668a);
        theme.colors.drop_target = komorebi_hex(0xe2d8c2);
    }
    window.refresh();
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

pub struct App {
    db: Database,
    accounts: Vec<Account>,
    folders: Vec<Folder>,
    selected_folder_id: Option<String>,
    messages: Vec<MessageRow>,
    selected_message: Option<MessageDetail>,
    panel: Panel,

    // New-message composer state.
    to_input: Entity<InputState>,
    subject_input: Entity<InputState>,
    body_input: Entity<InputState>,
    compose_status: Option<String>,
    composer_attachments: Vec<PendingAttachment>,
    composer_drag_over: bool,

    // Account setup state.
    setup_email: Entity<InputState>,
    setup_name: Entity<InputState>,
    setup_password: Entity<InputState>,
    setup_imap_host: Entity<InputState>,
    setup_imap_port: Entity<InputState>,
    setup_smtp_host: Entity<InputState>,
    setup_smtp_port: Entity<InputState>,
    setup_imap_security: SetupSecurity,
    setup_smtp_security: SetupSecurity,
    setup_client_id: Entity<InputState>,
    setup_client_secret: Entity<InputState>,
    setup_redirect_uri: Entity<InputState>,
    setup_discovered: Option<DiscoveredSettings>,
    setup_busy: bool,
    setup_status: Option<String>,

    // Sync state.
    sync_status: Option<String>,
    syncing: bool,

    // AI state.
    ai: Ai,
    ai_config: AiConfig,
    ai_model: Entity<InputState>,
    ai_base_url: Entity<InputState>,
    ai_output: Option<String>,
    ai_error: Option<String>,
    ai_busy: bool,
    ai_tone: String,
    ai_reply_draft: Option<String>,

    // Search + navigation.
    search_input: Entity<InputState>,
    search_query: String,
    _search_subscription: Subscription,
    focus_handle: FocusHandle,

    // Layout state.
    sidebar_visible: bool,
    ai_panel_visible: bool,
    list_width: Pixels,
    drag_over_folder_id: Option<String>,
    focused_tab: usize,
    toast: Option<String>,
    undo: Option<UndoReminder>,
    reminder_ids: Vec<String>,
    preview: Option<AttachmentPreview>,
    thread_messages: Vec<MessageDetail>,
    expanded_ids: Vec<String>,

    // Inline reply composer.
    reply_input: Entity<InputState>,
    reply_open: bool,
    reply_mode: ReplyMode,
    reply_to: String,
    reply_subject: String,
}

impl App {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("ctrl-n", NewMessage, Some(CONTEXT)),
            KeyBinding::new("ctrl-k", FocusSearch, Some(CONTEXT)),
            KeyBinding::new("ctrl-b", ToggleSidebar, Some(CONTEXT)),
            KeyBinding::new("ctrl-shift-t", ToggleTheme, Some(CONTEXT)),
            KeyBinding::new("j", NextMessage, Some(CONTEXT)),
            KeyBinding::new("k", PrevMessage, Some(CONTEXT)),
            KeyBinding::new("a", ArchiveMessage, Some(CONTEXT)),
            KeyBinding::new("s", StarMessage, Some(CONTEXT)),
            KeyBinding::new("r", ReplyMessage, Some(CONTEXT)),
            KeyBinding::new("f", ForwardMessage, Some(CONTEXT)),
        ]);

        let db = open_database();
        let _ = mail::seed_local_demo(&db);

        let ai = Ai::load(&db).unwrap_or_else(|_| Ai::load(&db).expect("ollama default must load"));
        let ai_config = ai.config().clone();

        // Persisted AI tone preference (falls back to Professional).
        let ai_tone = db
            .get_setting("ai.tone")
            .ok()
            .flatten()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "Professional".to_string());

        let to_input = cx.new(|cx| InputState::new(window, cx).placeholder("To (comma separated)"));
        let subject_input = cx.new(|cx| InputState::new(window, cx).placeholder("Subject"));
        let body_input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("Write your message…")
        });
        let reply_input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("Write your reply…")
        });
        let setup_email = cx.new(|cx| InputState::new(window, cx).placeholder("Email address"));
        let setup_name =
            cx.new(|cx| InputState::new(window, cx).placeholder("Display name (optional)"));
        let setup_password = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Password (stored in Credential Manager)")
        });
        let setup_imap_host = cx
            .new(|cx| InputState::new(window, cx).placeholder("IMAP host (e.g. mail.example.com)"));
        let setup_imap_port = cx.new(|cx| InputState::new(window, cx).placeholder("993"));
        let setup_smtp_host = cx
            .new(|cx| InputState::new(window, cx).placeholder("SMTP host (e.g. mail.example.com)"));
        let setup_smtp_port = cx.new(|cx| InputState::new(window, cx).placeholder("587"));
        let setup_client_id =
            cx.new(|cx| InputState::new(window, cx).placeholder("OAuth client id"));
        let setup_client_secret =
            cx.new(|cx| InputState::new(window, cx).placeholder("OAuth client secret"));
        let setup_redirect_uri = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Redirect URI (default http://127.0.0.1:8765/callback)")
        });
        let ai_model = cx.new(|cx| InputState::new(window, cx).placeholder("Model"));
        let ai_base_url =
            cx.new(|cx| InputState::new(window, cx).placeholder("Base URL (with /v1)"));
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search mail, people, labels…"));

        // Pre-fill AI inputs from the loaded config.
        ai_model.update(cx, |s, cx| s.set_value(ai_config.model.clone(), window, cx));
        ai_base_url.update(cx, |s, cx| {
            s.set_value(ai_config.base_url.clone(), window, cx)
        });

        // Live search: update the message list as the user types.
        let _search_subscription = cx.subscribe(&search_input, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.search_query = this.search_input.read(cx).value().to_string();
                this.reload_messages();
                cx.notify();
            }
        });

        let focus_handle = cx.focus_handle();
        {
            let handle = focus_handle.clone();
            window.defer(cx, move |window, _cx| {
                handle.focus(window);
            });
        }

        let mut this = Self {
            db,
            accounts: Vec::new(),
            folders: Vec::new(),
            selected_folder_id: None,
            messages: Vec::new(),
            selected_message: None,
            panel: Panel::Mail,
            to_input,
            subject_input,
            body_input,
            compose_status: None,
            composer_attachments: Vec::new(),
            composer_drag_over: false,
            setup_email,
            setup_name,
            setup_password,
            setup_imap_host,
            setup_imap_port,
            setup_smtp_host,
            setup_smtp_port,
            setup_imap_security: SetupSecurity::Tls,
            setup_smtp_security: SetupSecurity::StartTls,
            setup_client_id,
            setup_client_secret,
            setup_redirect_uri,
            setup_discovered: None,
            setup_busy: false,
            setup_status: None,
            sync_status: None,
            syncing: false,
            ai,
            ai_config,
            ai_model,
            ai_base_url,
            ai_output: None,
            ai_error: None,
            ai_busy: false,
            ai_tone,
            ai_reply_draft: None,
            search_input,
            search_query: String::new(),
            _search_subscription,
            focus_handle,
            sidebar_visible: true,
            ai_panel_visible: true,
            list_width: px(360.0),
            drag_over_folder_id: None,
            focused_tab: 0,
            toast: None,
            undo: None,
            reminder_ids: Vec::new(),
            preview: None,
            thread_messages: Vec::new(),
            expanded_ids: Vec::new(),
            reply_input,
            reply_open: false,
            reply_mode: ReplyMode::Reply,
            reply_to: String::new(),
            reply_subject: String::new(),
        };
        this.refresh_accounts();
        this.refresh_folders();
        this.select_first_folder();

        apply_komorebi_theme(window, cx);
        this
    }

    // ---------------------------------------------------------------------
    // Data helpers
    // ---------------------------------------------------------------------
    fn refresh_accounts(&mut self) {
        self.accounts = self.db.list_accounts().unwrap_or_default();
    }

    fn refresh_folders(&mut self) {
        self.folders = self
            .accounts
            .iter()
            .flat_map(|a| self.db.list_folders(&a.id).unwrap_or_default())
            .collect();
    }

    fn select_first_folder(&mut self) {
        if let Some(folder) = self.folders.first() {
            self.selected_folder_id = Some(folder.id.clone());
            self.reload_messages();
        }
    }

    fn account_messages(&self) -> Vec<MessageRow> {
        match self.accounts.first() {
            Some(a) => self
                .db
                .list_account_messages(&a.id, 1000)
                .unwrap_or_default(),
            None => Vec::new(),
        }
    }

    fn reload_messages(&mut self) {
        let now = chrono::Utc::now().timestamp();
        let _ = self.db.delete_expired_snoozes(now);

        let active = self.db.list_active_reminders(now).unwrap_or_default();
        self.reminder_ids = active.iter().map(|r| r.message_id.clone()).collect();
        let snoozed_ids: Vec<String> = active
            .iter()
            .filter(|r| r.kind == "snooze")
            .map(|r| r.message_id.clone())
            .collect();
        let inbox_id = self
            .folders
            .iter()
            .find(|f| f.special_use.as_deref() == Some("inbox"))
            .map(|f| f.id.clone());

        self.messages = match self.selected_folder_id.as_deref() {
            Some(PRIORITY) => self.db.list_flagged_messages(500, 0).unwrap_or_default(),
            Some(SNOOZED) => self.db.list_snoozed_messages(now).unwrap_or_default(),
            Some(VIEW_MENTIONS) => Vec::new(),
            Some(id) if id.starts_with(LABEL_PREFIX) => {
                let label = &id[LABEL_PREFIX.len()..];
                self.account_messages()
                    .into_iter()
                    .filter(|m| m.ai_category.as_deref() == Some(label))
                    .collect()
            }
            Some(VIEW_UNREAD) => self
                .account_messages()
                .into_iter()
                .filter(|m| !m.is_read)
                .collect(),
            Some(VIEW_ATTACHMENTS) => {
                let with_att = self
                    .db
                    .list_message_ids_with_attachments()
                    .unwrap_or_default();
                self.account_messages()
                    .into_iter()
                    .filter(|m| with_att.contains(&m.id))
                    .collect()
            }
            Some(_) if !self.search_query.trim().is_empty() => self
                .db
                .search_messages(&self.search_query, 500)
                .unwrap_or_default(),
            Some(id) => self.db.list_messages(id, 500, 0).unwrap_or_default(),
            None => Vec::new(),
        };

        // Snoozed messages leave the inbox and return when due.
        if self.search_query.trim().is_empty()
            && self.selected_folder_id.as_deref() == inbox_id.as_deref()
        {
            self.messages.retain(|m| !snoozed_ids.contains(&m.id));
        }

        // Focused vs Other: Focused surfaces unread first, Other surfaces read.
        if self.focused_tab == 0 {
            self.messages.sort_by(|a, b| {
                a.is_read
                    .cmp(&b.is_read)
                    .then(b.date_received.cmp(&a.date_received))
            });
        } else {
            self.messages.sort_by(|a, b| {
                b.is_read
                    .cmp(&a.is_read)
                    .then(b.date_received.cmp(&a.date_received))
            });
        }
    }

    fn open_message(&mut self, id: &str) {
        self.selected_message = self.db.get_message_detail(id).unwrap_or(None);
        if let Some(preview) = self.preview.take() {
            let _ = std::fs::remove_file(&preview.path);
        }
        self.thread_messages = match &self.selected_message {
            Some(m) => match &m.thread_id {
                Some(tid) => self.db.list_thread_messages(tid).unwrap_or_default(),
                None => vec![m.clone()],
            },
            None => Vec::new(),
        };
        self.expanded_ids = self
            .selected_message
            .as_ref()
            .map(|m| vec![m.id.clone()])
            .unwrap_or_default();
        if self.selected_message.is_some() {
            let _ = self.db.mark_read(id, true);
        }
    }

    fn current_view_label(&self) -> String {
        match self.selected_folder_id.as_deref() {
            Some(PRIORITY) => "Priority".to_string(),
            Some(SNOOZED) => "Snoozed".to_string(),
            Some(VIEW_UNREAD) => "Unread".to_string(),
            Some(VIEW_ATTACHMENTS) => "Has attachments".to_string(),
            Some(VIEW_MENTIONS) => "Mentions".to_string(),
            Some(id) if id.starts_with(LABEL_PREFIX) => id[LABEL_PREFIX.len()..].to_string(),
            Some(id) => self
                .folders
                .iter()
                .find(|f| f.id == id)
                .map(|f| f.display_name.clone().unwrap_or_else(|| f.name.clone()))
                .unwrap_or_else(|| "Inbox".to_string()),
            None => "Inbox".to_string(),
        }
    }

    fn folder_icon(folder: &Folder) -> IconName {
        match folder.special_use.as_deref() {
            Some("inbox") => IconName::Inbox,
            Some("sent") => IconName::ArrowRight,
            Some("drafts") => IconName::File,
            Some("archive") => IconName::FolderClosed,
            Some("trash") => IconName::Delete,
            Some("junk") => IconName::TriangleAlert,
            _ => IconName::Folder,
        }
    }

    fn selected_message_id(&self) -> Option<&str> {
        self.selected_message.as_ref().map(|m| m.id.as_str())
    }

    fn select_relative(&mut self, delta: isize) {
        if self.messages.is_empty() {
            return;
        }
        let current = self
            .selected_message_id()
            .and_then(|id| self.messages.iter().position(|m| m.id == id));
        let next = match current {
            Some(ix) => (ix as isize + delta).clamp(0, self.messages.len() as isize - 1) as usize,
            None => 0,
        };
        if let Some(msg) = self.messages.get(next) {
            let id = msg.id.clone();
            self.open_message(&id);
        }
    }

    fn user_profile(&self) -> (String, String) {
        match self.accounts.first() {
            Some(a) if a.provider_kind != "local" => (
                a.display_name.clone().unwrap_or_else(|| a.email.clone()),
                a.email.clone(),
            ),
            _ => (
                "Haruto Sato".to_string(),
                "haruto.sato@komorebi.com".to_string(),
            ),
        }
    }

    fn set_toast(&mut self, msg: &str, cx: &mut Context<Self>) {
        self.toast = Some(msg.to_string());
        cx.notify();
    }

    // ---------------------------------------------------------------------
    // Mail actions
    // ---------------------------------------------------------------------
    fn new_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_composer(window, cx);
        self.panel = Panel::Compose;
        cx.notify();
    }

    fn clear_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.compose_status = None;
        self.composer_attachments.clear();
        self.composer_drag_over = false;
        self.to_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.subject_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.body_input
            .update(cx, |s, cx| s.set_value("", window, cx));
    }

    fn clear_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reply_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.reply_to = String::new();
        self.reply_subject = String::new();
        self.ai_reply_draft = None;
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        cx.notify();
    }

    fn toggle_ai_panel(&mut self, cx: &mut Context<Self>) {
        self.ai_panel_visible = !self.ai_panel_visible;
        cx.notify();
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.search_input.read(cx).focus_handle(cx);
        handle.focus(window);
    }

    fn toggle_selected_flag(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_message_id().map(str::to_string) else {
            return;
        };
        self.toggle_flag(&id, cx);
    }

    fn archive_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_message_id().map(str::to_string) else {
            return;
        };
        self.archive_message(&id, cx);
    }

    fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_message_id().map(str::to_string) else {
            return;
        };
        self.delete_message(&id, cx);
    }

    /// Snooze the selected message until `due_at`, hiding it from the inbox.
    fn snooze_message(&mut self, id: &str, label: &str, due_at: i64, cx: &mut Context<Self>) {
        let Some(detail) = self.db.get_message_detail(id).ok().flatten() else {
            return;
        };
        let previous = self.db.get_reminder(id).ok().flatten();
        let _ = self.db.mark_read(id, true);
        let _ = self
            .db
            .upsert_reminder(&detail.account_id, id, "snooze", Some(label), due_at);
        self.undo = Some(UndoReminder {
            message_id: id.to_string(),
            previous,
        });
        self.reload_messages();
        self.set_toast(&format!("Snoozed until {label}"), cx);
        cx.notify();
    }

    /// Schedule a follow-up reminder (defaults to tomorrow) on the selected message.
    fn schedule_followup(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_message_id().map(str::to_string) else {
            self.set_toast("Select a message to schedule a follow-up.", cx);
            return;
        };
        let Some(detail) = self.db.get_message_detail(&id).ok().flatten() else {
            return;
        };
        let previous = self.db.get_reminder(&id).ok().flatten();
        let due_at = chrono::Utc::now().timestamp() + 24 * 3600;
        let _ = self.db.upsert_reminder(
            &detail.account_id,
            &id,
            "follow_up",
            Some("tomorrow"),
            due_at,
        );
        self.undo = Some(UndoReminder {
            message_id: id.clone(),
            previous,
        });
        self.reload_messages();
        self.set_toast("Follow-up scheduled for tomorrow", cx);
        cx.notify();
    }

    /// Undo the last snooze/follow-up action, restoring the prior reminder.
    fn undo_last(&mut self, cx: &mut Context<Self>) {
        let Some(undo) = self.undo.take() else {
            return;
        };
        if let Some(prev) = &undo.previous {
            let _ = self.db.upsert_reminder(
                &prev.account_id,
                &prev.message_id,
                &prev.kind,
                prev.label.as_deref(),
                prev.due_at,
            );
        } else {
            let _ = self.db.delete_reminder(&undo.message_id);
        }
        self.toast = Some("Undone".to_string());
        self.reload_messages();
        cx.notify();
    }

    fn move_selected_to_folder(
        &mut self,
        message_id: &str,
        folder_id: &str,
        cx: &mut Context<Self>,
    ) {
        self.move_to_folder(message_id, folder_id, cx);
    }

    fn open_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected_message_id().map(str::to_string) else {
            return;
        };
        self.reply_to(&id, window, cx);
    }

    fn open_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected_message_id().map(str::to_string) else {
            return;
        };
        self.forward_to(&id, window, cx);
    }

    /// Toggle the star flag on a specific message.
    fn toggle_flag(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(msg) = self.db.get_message_detail(id).ok().flatten() else {
            return;
        };
        let new_flag = !msg.is_flagged;
        let _ = self.db.mark_flagged(id, new_flag);
        if self.selected_message_id() == Some(id) {
            self.selected_message = Some(MessageDetail {
                is_flagged: new_flag,
                ..msg
            });
        }
        self.reload_messages();
        cx.notify();
    }

    fn set_message_read(&mut self, id: &str, read: bool, cx: &mut Context<Self>) {
        let _ = self.db.mark_read(id, read);
        if self.selected_message_id() == Some(id) {
            self.selected_message = self.db.get_message_detail(id).ok().flatten();
        }
        self.reload_messages();
        cx.notify();
    }

    fn archive_message(&mut self, id: &str, cx: &mut Context<Self>) {
        let target = self
            .folders
            .iter()
            .find(|f| f.special_use.as_deref() == Some("archive"));
        match target {
            Some(target) => {
                let _ = self.db.move_message(id, &target.id);
            }
            None => {
                let _ = self.db.mark_read(id, true);
            }
        }
        if self.selected_message_id() == Some(id) {
            self.selected_message = None;
        }
        self.reload_messages();
        cx.notify();
    }

    fn delete_message(&mut self, id: &str, cx: &mut Context<Self>) {
        let target = self
            .folders
            .iter()
            .find(|f| f.special_use.as_deref() == Some("trash"));
        match target {
            Some(target) => {
                let _ = self.db.move_message(id, &target.id);
            }
            None => {
                let _ = self.db.mark_read(id, true);
            }
        }
        if self.selected_message_id() == Some(id) {
            self.selected_message = None;
        }
        self.reload_messages();
        cx.notify();
    }

    fn move_to_folder(&mut self, message_id: &str, folder_id: &str, cx: &mut Context<Self>) {
        let _ = self.db.move_message(message_id, folder_id);
        if self.selected_message_id() == Some(message_id) {
            self.selected_message = None;
        }
        self.drag_over_folder_id = None;
        self.reload_messages();
        cx.notify();
    }

    fn reply_to(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(msg) = self.db.get_message_detail(id).ok().flatten() else {
            return;
        };
        let to = msg.sender_email.clone().unwrap_or_default();
        let subject = format!(
            "Re: {}",
            msg.subject.clone().unwrap_or_else(|| "(no subject)".into())
        );
        self.clear_reply(window, cx);
        self.reply_open = true;
        self.reply_mode = ReplyMode::Reply;
        self.reply_to = to;
        self.reply_subject = subject;
        cx.notify();
    }

    fn reply_all_to(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(msg) = self.db.get_message_detail(id).ok().flatten() else {
            return;
        };
        let mut to = vec![msg.sender_email.clone().unwrap_or_default()];
        if let Some(Ok(ccs)) = msg
            .cc_addresses
            .as_deref()
            .map(serde_json::from_str::<Vec<String>>)
        {
            to.extend(ccs);
        }
        let subject = format!(
            "Re: {}",
            msg.subject.clone().unwrap_or_else(|| "(no subject)".into())
        );
        self.clear_reply(window, cx);
        self.reply_open = true;
        self.reply_mode = ReplyMode::Reply;
        self.reply_to = to
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        self.reply_subject = subject;
        cx.notify();
    }

    fn forward_to(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(msg) = self.db.get_message_detail(id).ok().flatten() else {
            return;
        };
        let subject = format!(
            "Fwd: {}",
            msg.subject.clone().unwrap_or_else(|| "(no subject)".into())
        );
        self.clear_reply(window, cx);
        self.reply_open = true;
        self.reply_mode = ReplyMode::Reply;
        self.reply_to = String::new();
        self.reply_subject = subject;
        cx.notify();
    }

    fn add_attachments(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        for path in paths.paths() {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "attachment".to_string());
            match std::fs::read(path) {
                Ok(data) => {
                    self.composer_attachments.push(PendingAttachment {
                        size_bytes: data.len() as i64,
                        name,
                        data,
                    });
                }
                Err(e) => {
                    log::warn!("could not read dropped file {name}: {e}");
                }
            }
        }
        self.composer_drag_over = false;
        cx.notify();
    }

    // ---------------------------------------------------------------------
    // Inline reply composer
    // ---------------------------------------------------------------------
    fn send_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let body = self.reply_input.read(cx).value().to_string();
        if body.trim().is_empty() {
            self.set_toast("Write a reply first.", cx);
            return false;
        }
        let to: Vec<String> = if self.reply_to.trim().is_empty() {
            Vec::new()
        } else {
            self.reply_to
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        };
        let subject = self.reply_subject.clone();
        let attachments: Vec<mail::smtp::Attachment> = self
            .composer_attachments
            .iter()
            .map(|a| mail::smtp::Attachment {
                filename: a.name.clone(),
                content_type: mail::smtp::guess_content_type(&a.name).to_string(),
                data: a.data.clone(),
            })
            .collect();
        let account = self.accounts.first().cloned();

        // Clear the reply composer optimistically; the send owns its copies.
        self.reply_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.composer_attachments.clear();
        self.reply_to = String::new();
        self.reply_subject = String::new();
        self.ai_reply_draft = None;
        cx.notify();

        let db = self.db.clone();
        let entity = cx.entity();
        cx.spawn(async move |_weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match account.as_ref() {
                        Some(a) => perform_send(&db, a, &to, &subject, &body, &attachments),
                        None => Ok(()),
                    }
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                match result {
                    Ok(()) => this.set_toast("Reply sent ✓", cx),
                    Err(e) => this.set_toast(&format!("Send failed: {e}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
        true
    }

    fn run_ai_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(msg) = self.selected_message.clone() else {
            return;
        };
        self.clear_reply(window, cx);
        self.reply_open = true;
        self.reply_mode = ReplyMode::Reply;
        self.reply_to = msg.sender_email.clone().unwrap_or_default();
        self.reply_subject = format!(
            "Re: {}",
            msg.subject.clone().unwrap_or_else(|| "(no subject)".into())
        );
        self.ai_busy = true;
        self.ai_error = None;
        cx.notify();

        let sender = msg.sender_name.clone();
        let subj = msg.subject.clone();
        let body = msg.body_text.clone().unwrap_or_default();
        let message_id = msg.id.clone();
        let account_id = msg.account_id.clone();
        let ai = self.ai.clone();
        let db = self.db.clone();
        let tone = self.ai_tone.clone();
        let entity = cx.entity();

        cx.spawn(async move |_weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    perform_ai(
                        &ai,
                        &db,
                        AiKind::DraftReply,
                        &account_id,
                        &message_id,
                        sender.as_deref(),
                        subj.as_deref(),
                        &body,
                        &tone,
                    )
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                this.ai_busy = false;
                match result {
                    Ok(text) => this.ai_reply_draft = Some(text),
                    Err(e) => this.ai_error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---------------------------------------------------------------------
    // Context menus
    // ---------------------------------------------------------------------
    fn message_context_menu(
        &self,
        entity: Entity<Self>,
        message_id: String,
        is_read: bool,
        is_flagged: bool,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let current_folder_id = self
            .messages
            .iter()
            .find(|m| m.id == message_id)
            .map(|m| m.folder_id.clone());
        let folders: Vec<(String, String, IconName)> = self
            .folders
            .iter()
            .filter(|f| Some(f.id.as_str()) != current_folder_id.as_deref())
            .map(|f| {
                (
                    f.id.clone(),
                    f.display_name.clone().unwrap_or_else(|| f.name.clone()),
                    Self::folder_icon(f),
                )
            })
            .collect();

        move |menu, window, cx| {
            let menu = menu
                .item(
                    PopupMenuItem::new("Reply")
                        .icon(IconName::ArrowLeft)
                        .on_click({
                            let entity = entity.clone();
                            let id = message_id.clone();
                            move |_, window, cx| {
                                let id = id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    this.reply_to(&id, window, cx);
                                });
                            }
                        }),
                )
                .item(
                    PopupMenuItem::new("Reply all")
                        .icon(IconName::ArrowLeft)
                        .on_click({
                            let entity = entity.clone();
                            let id = message_id.clone();
                            move |_, window, cx| {
                                let id = id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    this.reply_all_to(&id, window, cx);
                                });
                            }
                        }),
                )
                .item(
                    PopupMenuItem::new("Forward")
                        .icon(IconName::ArrowRight)
                        .on_click({
                            let entity = entity.clone();
                            let id = message_id.clone();
                            move |_, window, cx| {
                                let id = id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    this.forward_to(&id, window, cx);
                                });
                            }
                        }),
                )
                .separator()
                .item(
                    PopupMenuItem::new("Archive")
                        .icon(IconName::FolderClosed)
                        .on_click({
                            let entity = entity.clone();
                            let id = message_id.clone();
                            move |_, _, cx| {
                                let id = id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    this.archive_message(&id, cx);
                                });
                            }
                        }),
                )
                .item(
                    PopupMenuItem::new(if is_flagged { "Unstar" } else { "Star" })
                        .icon(if is_flagged {
                            IconName::StarOff
                        } else {
                            IconName::Star
                        })
                        .on_click({
                            let entity = entity.clone();
                            let id = message_id.clone();
                            move |_, _, cx| {
                                let id = id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    this.toggle_flag(&id, cx);
                                });
                            }
                        }),
                )
                .item(
                    PopupMenuItem::new(if is_read {
                        "Mark as unread"
                    } else {
                        "Mark as read"
                    })
                    .icon(if is_read {
                        IconName::EyeOff
                    } else {
                        IconName::Eye
                    })
                    .on_click({
                        let entity = entity.clone();
                        let id = message_id.clone();
                        let target_read = is_read;
                        move |_, _, cx| {
                            let id = id.clone();
                            cx.update_entity(&entity, move |this, cx| {
                                this.set_message_read(&id, !target_read, cx);
                            });
                        }
                    }),
                )
                .separator();

            let menu =
                menu.submenu_with_icon(Some(Icon::new(IconName::Folder)), "Move to", window, cx, {
                    let folders = folders.clone();
                    let entity = entity.clone();
                    let message_id = message_id.clone();
                    move |submenu, _window, _cx| {
                        let mut m = submenu;
                        for (fid, label, icon) in &folders {
                            m = m.item(
                                PopupMenuItem::new(SharedString::from(label.clone()))
                                    .icon(Icon::new(icon.clone()))
                                    .on_click({
                                        let entity = entity.clone();
                                        let mid = message_id.clone();
                                        let fid = fid.clone();
                                        move |_, _, cx| {
                                            let mid = mid.clone();
                                            let fid = fid.clone();
                                            cx.update_entity(&entity, move |this, cx| {
                                                this.move_to_folder(&mid, &fid, cx);
                                            });
                                        }
                                    }),
                            );
                        }
                        m
                    }
                });

            menu.separator().item(
                PopupMenuItem::new("Delete")
                    .icon(IconName::Delete)
                    .on_click({
                        let entity = entity.clone();
                        let id = message_id.clone();
                        move |_, _, cx| {
                            let id = id.clone();
                            cx.update_entity(&entity, move |this, cx| {
                                this.delete_message(&id, cx);
                            });
                        }
                    }),
            )
        }
    }

    fn folder_context_menu(
        &self,
        entity: Entity<Self>,
        folder: Folder,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let folder_id = folder.id.clone();
        let special = folder.special_use.clone();
        let is_trash = special.as_deref() == Some("trash");
        let is_custom = special.is_none();
        let trash_id = self
            .folders
            .iter()
            .find(|f| f.special_use.as_deref() == Some("trash"))
            .map(|f| f.id.clone());

        move |menu, _window, _cx| {
            let menu = menu
                .item(
                    PopupMenuItem::new("Open")
                        .icon(IconName::FolderOpen)
                        .on_click({
                            let entity = entity.clone();
                            let id = folder_id.clone();
                            move |_, _, cx| {
                                let id = id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    this.selected_folder_id = Some(id);
                                    this.selected_message = None;
                                    this.reload_messages();
                                    cx.notify();
                                });
                            }
                        }),
                )
                .item(
                    PopupMenuItem::new("Mark all read")
                        .icon(IconName::Check)
                        .on_click({
                            let entity = entity.clone();
                            let id = folder_id.clone();
                            move |_, _, cx| {
                                let id = id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    let _ = this.db.mark_folder_read(&id);
                                    this.reload_messages();
                                    cx.notify();
                                });
                            }
                        }),
                );

            if is_trash {
                menu.separator().item(
                    PopupMenuItem::new("Empty trash")
                        .icon(IconName::Delete)
                        .on_click({
                            let entity = entity.clone();
                            let id = folder_id.clone();
                            move |_, _, cx| {
                                let id = id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    let _ = this.db.delete_all_messages(&id);
                                    this.selected_message = None;
                                    this.reload_messages();
                                    cx.notify();
                                });
                            }
                        }),
                )
            } else if is_custom {
                menu.separator().item(
                    PopupMenuItem::new("Delete folder")
                        .icon(IconName::Delete)
                        .on_click({
                            let entity = entity.clone();
                            let id = folder_id.clone();
                            move |_, _, cx| {
                                let id = id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    let _ = this.db.delete_folder(&id);
                                    this.refresh_folders();
                                    this.selected_message = None;
                                    if this.selected_folder_id.as_deref() == Some(id.as_str()) {
                                        this.selected_folder_id = None;
                                    }
                                    this.reload_messages();
                                    cx.notify();
                                });
                            }
                        }),
                )
            } else {
                menu.separator().item(
                    PopupMenuItem::new("Empty folder")
                        .icon(IconName::Delete)
                        .on_click({
                            let entity = entity.clone();
                            let id = folder_id.clone();
                            let trash_id = trash_id.clone();
                            move |_, _, cx| {
                                let id = id.clone();
                                let trash_id = trash_id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    if let Some(trash) = &trash_id {
                                        let _ = this.db.move_all_messages(&id, trash);
                                    }
                                    this.selected_message = None;
                                    this.reload_messages();
                                    cx.notify();
                                });
                            }
                        }),
                )
            }
        }
    }

    // ---------------------------------------------------------------------
    // Title bar
    // ---------------------------------------------------------------------
    fn titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let is_dark = cx.theme().mode.is_dark();
        let entity = cx.entity();
        let ai_visible = self.ai_panel_visible;

        TitleBar::new().child(
            h_flex()
                .w_full()
                .h_full()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(22.0))
                        .h(px(22.0))
                        .rounded_full()
                        .bg(cx.theme().primary)
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(IconName::Sun)
                                .size_3p5()
                                .text_color(cx.theme().primary_foreground),
                        ),
                )
                .child(Label::new("Komorebi").font_semibold())
                .child(
                    div()
                        .text_xs()
                        .px_1p5()
                        .rounded_full()
                        .bg(cx.theme().muted)
                        .text_color(cx.theme().muted_foreground)
                        .child("AI"),
                )
                .child(div().flex_1())
                .child(
                    Button::new("title-ai")
                        .icon(if ai_visible {
                            IconName::PanelRightClose
                        } else {
                            IconName::PanelRightOpen
                        })
                        .ghost()
                        .tooltip("Toggle AI panel")
                        .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                                cx.update_entity(&entity, |this, cx| {
                                    this.toggle_ai_panel(cx);
                                });
                            }
                        }),
                )
                .child(
                    Button::new("theme-toggle")
                        .icon(if is_dark {
                            IconName::Sun
                        } else {
                            IconName::Moon
                        })
                        .ghost()
                        .tooltip("Toggle theme (Ctrl+Shift+T)")
                        .on_click(move |_, window, cx| {
                            let next = if is_dark {
                                ThemeMode::Light
                            } else {
                                ThemeMode::Dark
                            };
                            Theme::change(next, Some(window), cx);
                            apply_komorebi_theme(window, cx);
                        }),
                ),
        )
    }

    // ---------------------------------------------------------------------
    // Sidebar
    // ---------------------------------------------------------------------
    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let entity = cx.entity();
        let priority_unread = self
            .db
            .list_flagged_messages(500, 0)
            .map(|m| m.len())
            .unwrap_or(0) as i64;
        let unread_view = self
            .account_messages()
            .into_iter()
            .filter(|m| !m.is_read)
            .count() as i64;

        v_flex()
            .id("sidebar")
            .h_full()
            .w_full()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .p_2()
            .gap_1()
            .overflow_y_scroll()
            .child(
                Button::new("compose")
                    .label("Compose")
                    .icon(IconName::Plus)
                    .primary()
                    .w_full()
                    .h_8()
                    .tooltip("Compose a new message (Ctrl+N)")
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                this.new_message(window, cx);
                            });
                        }
                    }),
            )
            .child(
                Button::new("sidebar-sync")
                    .label(if self.syncing { "Syncing…" } else { "Sync" })
                    .icon(IconName::Globe)
                    .ghost()
                    .w_full()
                    .h_7()
                    .justify_start()
                    .disabled(self.syncing)
                    .tooltip("Sync all accounts")
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| this.sync_now(cx));
                        }
                    }),
            )
            .child(self.section_header(cx, "Mail"))
            .child(self.mailbox_item(cx, entity.clone(), "inbox", "Inbox", IconName::Inbox))
            .child(self.nav_item(
                cx,
                entity.clone(),
                "nav-priority",
                PRIORITY.to_string(),
                IconName::Star,
                "Priority",
                priority_unread,
                None,
            ))
            .child(self.mailbox_item(cx, entity.clone(), "sent", "Sent", IconName::ArrowRight))
            .child(self.mailbox_item(cx, entity.clone(), "drafts", "Drafts", IconName::File))
            .child(self.mailbox_item(
                cx,
                entity.clone(),
                "archive",
                "Archive",
                IconName::FolderClosed,
            ))
            .child(self.nav_item(
                cx,
                entity.clone(),
                "nav-snoozed",
                SNOOZED.to_string(),
                IconName::Calendar,
                "Snoozed",
                0,
                None,
            ))
            .child(self.mailbox_item(cx, entity.clone(), "trash", "Trash", IconName::Delete))
            .child(self.section_header_with_add(cx, entity.clone(), "Labels"))
            .child(self.labels_list(cx, entity.clone()))
            .child(self.section_header_with_add(cx, entity.clone(), "Views"))
            .child(self.nav_item(
                cx,
                entity.clone(),
                "view-unread",
                VIEW_UNREAD.to_string(),
                IconName::Eye,
                "Unread",
                unread_view,
                None,
            ))
            .child(self.nav_item(
                cx,
                entity.clone(),
                "view-attachments",
                VIEW_ATTACHMENTS.to_string(),
                IconName::File,
                "Has attachments",
                0,
                None,
            ))
            .child(self.nav_item(
                cx,
                entity.clone(),
                "view-mentions",
                VIEW_MENTIONS.to_string(),
                IconName::Asterisk,
                "Mentions",
                0,
                None,
            ))
            .child(div().flex_1())
            .child(self.storage_widget(cx))
            .child(
                Button::new("accounts")
                    .label("Accounts & Settings")
                    .icon(IconName::Settings)
                    .ghost()
                    .w_full()
                    .justify_start()
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                this.panel = Panel::Accounts;
                                cx.notify();
                            });
                        }
                    }),
            )
    }

    fn section_header(
        &self,
        cx: &mut Context<Self>,
        title: &'static str,
    ) -> impl IntoElement + use<> {
        h_flex()
            .px_2()
            .pt_3()
            .pb_1()
            .w_full()
            .text_xs()
            .font_medium()
            .text_color(cx.theme().muted_foreground)
            .child(SharedString::from(title))
    }

    fn section_header_with_add(
        &self,
        cx: &mut Context<Self>,
        _entity: Entity<Self>,
        title: &'static str,
    ) -> impl IntoElement + use<> {
        h_flex()
            .px_2()
            .pt_3()
            .pb_1()
            .w_full()
            .text_xs()
            .font_medium()
            .text_color(cx.theme().muted_foreground)
            .justify_between()
            .child(SharedString::from(title))
            .child(
                Button::new(SharedString::from(format!("add-{title}")))
                    .icon(IconName::Plus)
                    .ghost()
                    .xsmall()
                    .tooltip(SharedString::from(format!("Add {title}"))),
            )
    }

    fn mailbox_item(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        special_use: &str,
        label: &'static str,
        icon: IconName,
    ) -> AnyElement {
        match self
            .folders
            .iter()
            .find(|f| f.special_use.as_deref() == Some(special_use))
            .cloned()
        {
            Some(folder) => {
                let unread = folder.unread_count;
                let id = folder.id.clone();
                self.nav_item(
                    cx,
                    entity,
                    &format!("nav-{special_use}"),
                    id,
                    icon,
                    label,
                    unread,
                    Some(folder),
                )
            }
            None => div().into_any_element(),
        }
    }

    fn labels_list(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
    ) -> impl IntoElement + use<> {
        let mut list = v_flex().gap_1();
        for (name, color) in LABELS {
            let target = format!("{LABEL_PREFIX}{name}");
            list = list.child(self.label_row(cx, entity.clone(), name, *color, target));
        }
        list
    }

    fn label_row(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        label: &'static str,
        color: u32,
        target: String,
    ) -> AnyElement {
        let selected = self.selected_folder_id.as_deref() == Some(target.as_str());
        let selected_bg = cx.theme().sidebar_accent;
        let selected_fg = cx.theme().sidebar_accent_foreground;
        let hover_bg = cx.theme().muted;
        let dot = komorebi_hex(color);

        h_flex()
            .id(SharedString::from(format!("label-{label}")))
            .w_full()
            .px_2()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .gap_2()
            .items_center()
            .when(selected, |d| d.bg(selected_bg).text_color(selected_fg))
            .when(!selected, |d| d.hover(|s| s.bg(hover_bg)))
            .child(
                div()
                    .w(px(8.0))
                    .h(px(8.0))
                    .rounded_full()
                    .bg(dot)
                    .flex_none(),
            )
            .child(SharedString::from(label))
            .child(div().flex_1())
            .on_click({
                let entity = entity.clone();
                let t = target.clone();
                move |_, _, cx| {
                    let t = t.clone();
                    cx.update_entity(&entity, move |this, cx| {
                        this.selected_folder_id = Some(t);
                        this.selected_message = None;
                        this.search_query.clear();
                        this.reload_messages();
                        cx.notify();
                    });
                }
            })
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn nav_item(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        id: &str,
        target: String,
        icon: IconName,
        label: &'static str,
        unread: i64,
        context_folder: Option<Folder>,
    ) -> AnyElement {
        let selected = self.selected_folder_id.as_deref() == Some(target.as_str());
        let selected_bg = cx.theme().sidebar_accent;
        let selected_fg = cx.theme().sidebar_accent_foreground;
        let hover_bg = cx.theme().muted;
        let muted = cx.theme().muted_foreground;
        let label_owned = SharedString::from(label);

        let row = h_flex()
            .id(SharedString::from(id.to_string()))
            .w_full()
            .px_2()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .gap_2()
            .items_center()
            .when(selected, |d| d.bg(selected_bg).text_color(selected_fg))
            .when(!selected, |d| d.hover(|s| s.bg(hover_bg)))
            .child(
                Icon::new(icon)
                    .size_4()
                    .text_color(if selected { selected_fg } else { muted }),
            )
            .child(label_owned.clone())
            .child(div().flex_1())
            .children(unread_guard(unread, cx))
            .on_click({
                let entity = entity.clone();
                let t = target.clone();
                move |_, _, cx| {
                    let t = t.clone();
                    cx.update_entity(&entity, move |this, cx| {
                        this.selected_folder_id = Some(t);
                        this.selected_message = None;
                        this.search_query.clear();
                        this.reload_messages();
                        cx.notify();
                    });
                }
            });

        let row = match context_folder.as_ref() {
            Some(folder) => {
                let fid_drop = folder.id.clone();
                let fid_drag = folder.id.clone();
                row.on_drop::<DragMessage>({
                    let entity = entity.clone();
                    move |drag, _, cx| {
                        let mid = drag.message_id.clone();
                        let fid = fid_drop.clone();
                        cx.update_entity(&entity, move |this, cx| {
                            this.move_selected_to_folder(&mid, &fid, cx);
                        });
                    }
                })
                .on_drag_move::<DragMessage>({
                    let entity = entity.clone();
                    move |_, _, cx| {
                        let fid = fid_drag.clone();
                        cx.update_entity(&entity, move |this, cx| {
                            this.drag_over_folder_id = Some(fid);
                            cx.notify();
                        });
                    }
                })
            }
            None => row,
        };

        match context_folder {
            Some(folder) => row
                .context_menu(self.folder_context_menu(entity, folder))
                .into_any_element(),
            None => row.into_any_element(),
        }
    }

    fn storage_widget(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        v_flex()
            .id("storage")
            .m_1()
            .p_3()
            .rounded_lg()
            .bg(cx.theme().muted)
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .child(div().text_xs().font_medium().child("Storage"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("2.4 GB of 10 GB"),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h(px(4.0))
                    .rounded_full()
                    .bg(cx.theme().border)
                    .child(
                        div()
                            .h_full()
                            .w(px(56.0))
                            .rounded_full()
                            .bg(cx.theme().primary),
                    ),
            )
    }

    // ---------------------------------------------------------------------
    // Message list
    // ---------------------------------------------------------------------
    fn mail_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let list = self.message_list(cx).into_any_element();
        let reader = self.reader(cx).into_any_element();
        let list_width = self.list_width;
        let entity = cx.entity();

        h_resizable("mail")
            .on_resize({
                let entity = entity.clone();
                move |state, _, cx| {
                    if let Some(width) = state.read(cx).sizes().first().copied() {
                        cx.update_entity(&entity, |this, cx| {
                            this.list_width = width;
                            cx.notify();
                        });
                    }
                }
            })
            .child(
                resizable_panel()
                    .size(list_width)
                    .size_range(px(300.0)..px(560.0))
                    .child(list),
            )
            .child(resizable_panel().child(reader))
    }

    fn message_list(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted = cx.theme().muted_foreground;
        let item_count = self.messages.len();
        let search = self.search_input.clone();
        let entity = cx.entity();

        v_flex()
            .id("message-list")
            .h_full()
            .w_full()
            .child(
                // Search header.
                h_flex().w_full().px_3().py_2().gap_2().child(
                    Input::new(&search)
                        .small()
                        .w_full()
                        .prefix(Icon::new(IconName::Search).small().text_color(muted))
                        .suffix(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded_md()
                                .bg(cx.theme().muted)
                                .text_xs()
                                .text_color(muted)
                                .child("Ctrl K"),
                        ),
                ),
            )
            .child(
                // Focused / Other tabs + filter.
                h_flex()
                    .w_full()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(
                        TabBar::new("focused-tabs")
                            .pill()
                            .children([Tab::new().label("Focused"), Tab::new().label("Other")])
                            .selected_index(self.focused_tab)
                            .on_click({
                                let entity = entity.clone();
                                move |idx, _, cx| {
                                    let idx = *idx;
                                    cx.update_entity(&entity, move |this, cx| {
                                        this.focused_tab = idx;
                                        this.reload_messages();
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new("filter")
                            .label("Filter")
                            .icon(IconName::ChevronsUpDown)
                            .ghost()
                            .small()
                            .tooltip("Filter messages")
                            .dropdown_menu(|menu, _, _| {
                                menu.item(PopupMenuItem::new("Unread").icon(IconName::Eye))
                                    .item(PopupMenuItem::new("Has attachment").icon(IconName::File))
                                    .item(PopupMenuItem::new("Starred").icon(IconName::Star))
                                    .separator()
                                    .item(PopupMenuItem::new("From sender…").icon(IconName::User))
                            }),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .pb_1()
                    .items_center()
                    .justify_between()
                    .child(
                        Label::new(SharedString::from(self.current_view_label()))
                            .text_lg()
                            .font_semibold(),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(SharedString::from(format!("{} messages", item_count))),
                    ),
            )
            .child(if self.messages.is_empty() {
                self.empty_list(cx).into_any_element()
            } else {
                uniform_list(
                    "message-rows",
                    item_count,
                    cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                        let entity = cx.entity();
                        let mut items = Vec::with_capacity(range.len());
                        for ix in range {
                            if let Some(msg) = this.messages.get(ix) {
                                items.push(this.message_row(msg, entity.clone(), cx));
                            }
                        }
                        items
                    }),
                )
                .flex_1()
                .w_full()
                .into_any_element()
            })
    }

    fn empty_list(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let label = match self.selected_folder_id.as_deref() {
            Some(SNOOZED) => "Nothing snoozed. Snoozed conversations return here when due.",
            Some(VIEW_MENTIONS) => "No mentions found.",
            Some(VIEW_ATTACHMENTS) => "No messages with attachments.",
            Some(VIEW_UNREAD) => "You're all caught up.",
            Some(id) if id.starts_with(LABEL_PREFIX) => "No messages with this label yet.",
            _ => "No messages in this view.",
        };
        div()
            .id("empty")
            .flex_1()
            .w_full()
            .p_3()
            .text_color(cx.theme().muted_foreground)
            .child(SharedString::from(label))
    }

    fn message_row(
        &self,
        msg: &MessageRow,
        entity: Entity<Self>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.selected_message_id() == Some(msg.id.as_str());
        let msg_id = msg.id.clone();
        let sender = msg
            .sender_name
            .clone()
            .unwrap_or_else(|| msg.sender_email.clone().unwrap_or_default());
        let subject = msg
            .subject
            .clone()
            .unwrap_or_else(|| "(no subject)".to_string());
        let snippet = msg.snippet.clone().unwrap_or_default();
        let date = format_list_date(msg.date_received);
        let unread = !msg.is_read;
        let flagged = msg.is_flagged;
        let label = msg.ai_category.clone();

        let border = cx.theme().border;
        let list_hover = cx.theme().list_hover;
        let list_active = cx.theme().list_active;
        let primary = cx.theme().primary;
        let muted = cx.theme().muted_foreground;

        h_flex()
            .id(SharedString::from(format!("msg-{msg_id}")))
            .w_full()
            .pl_2()
            .pr_3()
            .py_2()
            .border_b_1()
            .border_color(border)
            .cursor_pointer()
            .gap_2()
            .relative()
            .when(selected, |d| d.bg(list_active))
            .when(!selected, |d| d.hover(|s| s.bg(list_hover)))
            .child(
                div()
                    .flex_none()
                    .w(px(8.0))
                    .items_center()
                    .justify_center()
                    .h_full()
                    .child(
                        Badge::new()
                            .dot()
                            .color(primary)
                            .when(!unread, |b| b.color(gpui::transparent_black())),
                    ),
            )
            .child(
                Avatar::new()
                    .name(SharedString::from(sender.clone()))
                    .small(),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .truncate()
                                    .when(unread, |d| d.font_semibold())
                                    .child(SharedString::from(sender.clone())),
                            )
                            .child(
                                h_flex()
                                    .flex_none()
                                    .gap_1()
                                    .items_center()
                                    .children(star_marker(flagged, cx))
                                    .children(if self.reminder_ids.contains(&msg.id) {
                                        Some(
                                            Icon::new(IconName::Calendar)
                                                .size_3p5()
                                                .text_color(cx.theme().warning)
                                                .into_any_element(),
                                        )
                                    } else {
                                        None
                                    })
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(muted)
                                            .child(SharedString::from(date)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .truncate()
                            .when(unread, |d| d.font_semibold())
                            .child(SharedString::from(subject)),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .text_color(muted)
                                    .child(SharedString::from(snippet)),
                            )
                            .children(label.map(|l| label_badge(cx, &l))),
                    ),
            )
            .on_click({
                let entity = entity.clone();
                let mid = msg_id.clone();
                move |_, _, cx| {
                    let id = mid.clone();
                    cx.update_entity(&entity, move |this, cx| {
                        this.open_message(&id);
                        cx.notify();
                    });
                }
            })
            .on_drag(
                DragMessage {
                    message_id: msg_id.clone(),
                },
                |drag, _, _, cx| {
                    cx.new(|_| DragMessagePreview {
                        message_id: drag.message_id.clone(),
                    })
                },
            )
            .context_menu(self.message_context_menu(entity, msg_id, msg.is_read, msg.is_flagged))
            .into_any_element()
    }

    // ---------------------------------------------------------------------
    // Reader
    // ---------------------------------------------------------------------
    fn reader(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let entity = cx.entity();
        if self.selected_message.is_none() {
            return div()
                .id("reader")
                .h_full()
                .w_full()
                .flex_col()
                .items_center()
                .justify_center()
                .text_color(cx.theme().muted_foreground)
                .child(
                    v_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Icon::new(IconName::Inbox)
                                .size_5()
                                .text_color(cx.theme().muted),
                        )
                        .child(Label::new("Select a conversation").text_lg().font_medium())
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("Messages appear here for focused reading."),
                        ),
                );
        }
        let msg = self.selected_message.clone().expect("checked above");

        let subject = msg
            .subject
            .clone()
            .unwrap_or_else(|| "(no subject)".to_string());
        let flagged = msg.is_flagged;
        let category = msg.ai_category.clone();
        let message_id = msg.id.clone();
        let toast = self.toast.clone();
        let reminder = self.db.get_reminder(&message_id).ok().flatten();
        let has_undo = self.undo.is_some();
        let mut tags = category_tags(cx, category.as_deref(), flagged);
        if let Some(r) = &reminder {
            let label = r.label.clone().unwrap_or_default();
            let text = match r.kind.as_str() {
                "snooze" => format!("Snoozed until {label}"),
                _ => format!("Follow up · {label}"),
            };
            tags.push(reminder_tag(cx, &text));
        }

        let scroll = v_flex()
            .id("reader-scroll")
            .flex_1()
            .w_full()
            .overflow_y_scroll()
            .p_4()
            .child(
                v_flex()
                    .gap_4()
                    .child(
                        // Subject + tags + star.
                        h_flex()
                            .w_full()
                            .gap_3()
                            .items_center()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_2xl()
                                            .font_semibold()
                                            .child(SharedString::from(subject)),
                                    )
                                    .child(h_flex().gap_1().items_center().children(tags)),
                            )
                            .child(
                                Icon::new(if flagged {
                                    IconName::Star
                                } else {
                                    IconName::StarOff
                                })
                                .size_4()
                                .text_color(if flagged {
                                    cx.theme().primary
                                } else {
                                    cx.theme().muted_foreground
                                }),
                            ),
                    )
                    .child(self.conversation_view(cx, entity.clone()))
                    .child(self.attachments_section(cx, entity.clone(), &message_id))
                    .children(self.preview_section(cx, entity.clone()))
                    .child(self.quick_actions(cx, entity.clone()))
                    .child(self.inline_composer(cx, entity.clone()))
                    .children(toast.map(|t| {
                        let mut row = h_flex()
                            .w_full()
                            .p_2()
                            .rounded_md()
                            .bg(cx.theme().muted)
                            .gap_2()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(SharedString::from(t)),
                            );
                        if has_undo {
                            row = row.child(
                                Button::new("toast-undo").label("Undo").small().on_click({
                                    let entity = entity.clone();
                                    move |_, _, cx| {
                                        cx.update_entity(&entity, |this, cx| {
                                            this.undo_last(cx);
                                        });
                                    }
                                }),
                            );
                        }
                        row.child(
                            Button::new("toast-dismiss")
                                .icon(IconName::Close)
                                .ghost()
                                .xsmall()
                                .on_click({
                                    let entity = entity.clone();
                                    move |_, _, cx| {
                                        cx.update_entity(&entity, |this, cx| {
                                            this.toast = None;
                                            cx.notify();
                                        });
                                    }
                                }),
                        )
                    })),
            );

        div()
            .id("reader")
            .h_full()
            .w_full()
            .flex_col()
            .child(self.reader_toolbar(cx, entity, flagged))
            .child(scroll)
    }

    // ---------------------------------------------------------------------
    // Conversation (thread) rendering
    // ---------------------------------------------------------------------
    fn conversation_view(&self, cx: &mut Context<Self>, entity: Entity<Self>) -> AnyElement {
        if self.thread_messages.len() > 1 {
            self.thread_view(cx, entity)
        } else {
            self.single_message_view(cx)
        }
    }

    fn single_message_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(msg) = self.selected_message.as_ref() else {
            return div().into_any_element();
        };
        let sender = msg
            .sender_name
            .clone()
            .unwrap_or_else(|| msg.sender_email.clone().unwrap_or_default());
        let sender_email = msg.sender_email.clone().unwrap_or_default();
        let date = format_full_date(msg.date_received);
        let to = msg.to_addresses.clone().unwrap_or_default();
        let unread = !msg.is_read;
        let body = msg.body_text.clone().unwrap_or_default();

        v_flex()
            .gap_4()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(Avatar::new().name(SharedString::from(sender.clone())))
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_0p5()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_baseline()
                                    .child(div().font_semibold().child(SharedString::from(sender)))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(SharedString::from(recipient_line(
                                                &to,
                                                &sender_email,
                                            ))),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(SharedString::from(date)),
                            ),
                    )
                    .child(h_flex().gap_1().children(if unread {
                        Some(
                            Badge::new()
                                .color(cx.theme().primary)
                                .child("Unread")
                                .into_any_element(),
                        )
                    } else {
                        None
                    })),
            )
            .child(render_body(&body))
            .into_any_element()
    }

    fn thread_view(&self, cx: &mut Context<Self>, entity: Entity<Self>) -> AnyElement {
        let count = self.thread_messages.len();
        v_flex()
            .id("thread-view")
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_medium()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(format!(
                        "{count} messages in this conversation"
                    ))),
            )
            .children(
                self.thread_messages
                    .iter()
                    .map(|m| self.thread_message_card(cx, entity.clone(), m))
                    .collect::<Vec<_>>(),
            )
            .into_any_element()
    }

    fn thread_message_card(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        msg: &MessageDetail,
    ) -> AnyElement {
        let expanded = self.expanded_ids.contains(&msg.id);
        let sender = msg
            .sender_name
            .clone()
            .unwrap_or_else(|| msg.sender_email.clone().unwrap_or_default());
        let date = format_full_date(msg.date_received);
        let body = msg.body_text.clone().unwrap_or_default();

        v_flex()
            .id(SharedString::from(format!("thread-msg-{}", msg.id)))
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .id(SharedString::from(format!("thread-head-{}", msg.id)))
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .cursor_pointer()
                    .on_click({
                        let entity = entity.clone();
                        let id = msg.id.clone();
                        move |_, _, cx| {
                            let id = id.clone();
                            cx.update_entity(&entity, move |this, cx| {
                                this.toggle_thread_message(&id, cx);
                            });
                        }
                    })
                    .child(
                        Avatar::new()
                            .name(SharedString::from(sender.clone()))
                            .small(),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .child(SharedString::from(sender)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(SharedString::from(date)),
                            ),
                    )
                    .child(
                        Icon::new(if expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size_3p5()
                        .text_color(cx.theme().muted_foreground),
                    ),
            )
            .children(if expanded {
                Some(
                    v_flex()
                        .px_3()
                        .pb_2()
                        .child(render_body(&body))
                        .into_any_element(),
                )
            } else {
                None
            })
            .into_any_element()
    }

    fn toggle_thread_message(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.expanded_ids.contains(&id.to_string()) {
            self.expanded_ids.retain(|x| x != id);
        } else {
            self.expanded_ids.push(id.to_string());
        }
        cx.notify();
    }

    fn reader_toolbar(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        flagged: bool,
    ) -> impl IntoElement + use<> {
        h_flex()
            .id("reader-toolbar")
            .w_full()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .gap_1()
            .child(
                Button::new("toolbar-back")
                    .icon(IconName::ChevronLeft)
                    .ghost()
                    .tooltip("Back to list")
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                this.selected_message = None;
                                cx.notify();
                            });
                        }
                    }),
            )
            .child(
                Button::new("toolbar-archive")
                    .icon(IconName::FolderClosed)
                    .ghost()
                    .tooltip("Archive (A)")
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| this.archive_selected(cx));
                        }
                    }),
            )
            .child(
                Button::new("toolbar-delete")
                    .icon(IconName::Delete)
                    .ghost()
                    .tooltip("Delete")
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| this.delete_selected(cx));
                        }
                    }),
            )
            .child(
                Button::new("toolbar-unread")
                    .icon(IconName::EyeOff)
                    .ghost()
                    .tooltip("Mark unread (U)")
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                if let Some(id) = this.selected_message_id().map(str::to_string) {
                                    this.set_message_read(&id, false, cx);
                                }
                            });
                        }
                    }),
            )
            .child(
                Button::new("toolbar-snooze")
                    .icon(IconName::Calendar)
                    .ghost()
                    .tooltip("Snooze")
                    .dropdown_menu({
                        let entity = entity.clone();
                        move |menu, _, _| {
                            let mut m = menu;
                            for (label, secs) in SNOOZE_OPTIONS {
                                m = m.item(
                                    PopupMenuItem::new(SharedString::from(*label)).on_click({
                                        let entity = entity.clone();
                                        move |_, _, cx| {
                                            let label = *label;
                                            let secs = *secs;
                                            cx.update_entity(&entity, move |this, cx| {
                                                if let Some(id) =
                                                    this.selected_message_id().map(str::to_string)
                                                {
                                                    this.snooze_message(
                                                        &id,
                                                        label,
                                                        chrono::Utc::now().timestamp() + secs,
                                                        cx,
                                                    );
                                                }
                                            });
                                        }
                                    }),
                                );
                            }
                            m
                        }
                    }),
            )
            .child(
                Button::new("toolbar-flag")
                    .icon(if flagged {
                        IconName::Star
                    } else {
                        IconName::StarOff
                    })
                    .ghost()
                    .tooltip("Star (S)")
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| this.toggle_selected_flag(cx));
                        }
                    }),
            )
            .child(div().flex_1())
            .child(
                Button::new("toolbar-reply")
                    .icon(IconName::ArrowLeft)
                    .ghost()
                    .tooltip("Reply (R)")
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            cx.update_entity(&entity, |this, cx| this.open_reply(window, cx));
                        }
                    }),
            )
            .child(
                Button::new("toolbar-forward")
                    .icon(IconName::ArrowRight)
                    .ghost()
                    .tooltip("Forward (F)")
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            cx.update_entity(&entity, |this, cx| this.open_forward(window, cx));
                        }
                    }),
            )
            .child(
                Button::new("toolbar-more")
                    .icon(IconName::EllipsisVertical)
                    .ghost()
                    .tooltip("More")
                    .dropdown_menu({
                        let entity = entity.clone();
                        let sender_email = self
                            .selected_message
                            .as_ref()
                            .and_then(|m| m.sender_email.clone())
                            .unwrap_or_default();
                        move |menu, _, _| {
                            let email = sender_email.clone();
                            menu.item(
                                PopupMenuItem::new("Mark as unread")
                                    .icon(IconName::EyeOff)
                                    .on_click({
                                        let entity = entity.clone();
                                        move |_, _, cx| {
                                            cx.update_entity(&entity, |this, cx| {
                                                if let Some(id) =
                                                    this.selected_message_id().map(str::to_string)
                                                {
                                                    this.set_message_read(&id, false, cx);
                                                }
                                            });
                                        }
                                    }),
                            )
                            .separator()
                            .item(
                                PopupMenuItem::new("Copy sender")
                                    .icon(IconName::Copy)
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            email.clone(),
                                        ));
                                    }),
                            )
                        }
                    }),
            )
    }

    fn quick_actions(
        &self,
        _cx: &mut Context<Self>,
        entity: Entity<Self>,
    ) -> impl IntoElement + use<> {
        h_flex()
            .gap_2()
            .flex_wrap()
            .child(
                Button::new("qa-reply")
                    .label("Reply")
                    .icon(IconName::ArrowLeft)
                    .outline()
                    .small()
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            cx.update_entity(&entity, |this, cx| this.open_reply(window, cx));
                        }
                    }),
            )
            .child(
                Button::new("qa-reply-all")
                    .label("Reply all")
                    .icon(IconName::ArrowLeft)
                    .outline()
                    .small()
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                if let Some(id) = this.selected_message_id().map(str::to_string) {
                                    this.reply_all_to(&id, window, cx);
                                }
                            });
                        }
                    }),
            )
            .child(
                Button::new("qa-forward")
                    .label("Forward")
                    .icon(IconName::ArrowRight)
                    .outline()
                    .small()
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            cx.update_entity(&entity, |this, cx| this.open_forward(window, cx));
                        }
                    }),
            )
            .child(
                Button::new("qa-ai-reply")
                    .label("AI Reply")
                    .icon(IconName::Bot)
                    .primary()
                    .small()
                    .on_click({
                        let entity = entity.clone();
                        move |_, window, cx| {
                            cx.update_entity(&entity, |this, cx| this.run_ai_reply(window, cx));
                        }
                    }),
            )
    }

    fn inline_composer(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
    ) -> impl IntoElement + use<> {
        let reply_input = self.reply_input.clone();
        let reply_to = self.reply_to.clone();
        let ai_draft = self.ai_reply_draft.clone();
        let mode = self.reply_mode;

        v_flex()
            .id("inline-composer")
            .mt_1()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .gap_1()
            .p_2()
            .child(
                TabBar::new("composer-tabs")
                    .underline()
                    .children([Tab::new().label("Reply"), Tab::new().label("Internal note")])
                    .selected_index(if mode == ReplyMode::Note { 1 } else { 0 })
                    .on_click({
                        let entity = entity.clone();
                        move |idx, _, cx| {
                            let idx = *idx;
                            cx.update_entity(&entity, move |this, cx| {
                                this.reply_mode = if idx == 1 {
                                    ReplyMode::Note
                                } else {
                                    ReplyMode::Reply
                                };
                                cx.notify();
                            });
                        }
                    }),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(if reply_to.is_empty() {
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Reply to sender")
                            .into_any_element()
                    } else {
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(SharedString::from(format!("To: {reply_to}")))
                            .into_any_element()
                    })
                    .child(
                        div()
                            .w_full()
                            .min_h(px(96.0))
                            .rounded_md()
                            .bg(cx.theme().input)
                            .child(Input::new(&reply_input).border_0()),
                    ),
            )
            .children(ai_draft.map(|text| self.ai_draft_bar(cx, entity.clone(), text)))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(self.formatting_bar(cx))
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                Button::new("discard-reply")
                                    .icon(IconName::Delete)
                                    .ghost()
                                    .tooltip("Discard")
                                    .on_click({
                                        let entity = entity.clone();
                                        move |_, window, cx| {
                                            cx.update_entity(&entity, |this, cx| {
                                                this.clear_reply(window, cx);
                                                this.reply_open = false;
                                                cx.notify();
                                            });
                                        }
                                    }),
                            )
                            .child(
                                Button::new("send-reply")
                                    .label("Send")
                                    .icon(IconName::ArrowRight)
                                    .primary()
                                    .small()
                                    .on_click({
                                        let entity = entity.clone();
                                        move |_, window, cx| {
                                            cx.update_entity(&entity, |this, cx| {
                                                let _ = this.send_reply(window, cx);
                                            });
                                        }
                                    }),
                            ),
                    ),
            )
    }

    fn ai_draft_bar(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        text: String,
    ) -> AnyElement {
        h_flex()
            .w_full()
            .p_2()
            .rounded_md()
            .bg(cx.theme().muted)
            .gap_2()
            .justify_between()
            .items_center()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("AI drafted a reply in your tone."),
            )
            .child(
                Button::new("insert-ai-draft")
                    .label("Insert")
                    .primary()
                    .small()
                    .on_click(move |_, window, cx| {
                        let text = text.clone();
                        cx.update_entity(&entity, move |this, cx| {
                            let value = this.reply_input.read(cx).value().to_string();
                            let merged = if value.trim().is_empty() {
                                text.clone()
                            } else {
                                format!("{value}\n\n{text}")
                            };
                            this.reply_input.update(cx, |s, cx| {
                                s.set_value(SharedString::from(merged), window, cx)
                            });
                            this.ai_reply_draft = None;
                            cx.notify();
                        });
                    }),
            )
            .into_any_element()
    }

    fn formatting_bar(&self, _cx: &mut Context<Self>) -> impl IntoElement + use<> {
        h_flex()
            .gap_0()
            .items_center()
            .child(format_button("Aa", "Format"))
            .child(format_button("B", "Bold"))
            .child(format_button("I", "Italic"))
            .child(format_button("U", "Underline"))
            .child(format_button("🔗", "Link"))
            .child(format_button("🖼", "Image"))
            .child(format_button("📎", "Attach"))
            .child(format_button("📅", "Schedule"))
    }

    // ---------------------------------------------------------------------
    // Attachment actions
    // ---------------------------------------------------------------------
    fn download_attachment(&mut self, att: Attachment, cx: &mut Context<Self>) {
        let db = self.db.clone();
        let entity = cx.entity();
        cx.spawn(async move |_weak, cx| {
            let result: anyhow::Result<PathBuf> = cx
                .background_executor()
                .spawn(async move {
                    let data = load_attachment_data(&db, &att)?;
                    let dir = dirs::download_dir()
                        .filter(|d| d.is_dir())
                        .unwrap_or_else(std::env::temp_dir);
                    write_blob(&dir, &att.display_name, &data)
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                match result {
                    Ok(path) => this.set_toast(&format!("Saved {}", path.display()), cx),
                    Err(e) => this.set_toast(&format!("Download failed: {e}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn download_all_attachments(&mut self, atts: Vec<Attachment>, cx: &mut Context<Self>) {
        let db = self.db.clone();
        let entity = cx.entity();
        cx.spawn(async move |_weak, cx| {
            let result: anyhow::Result<usize> = cx
                .background_executor()
                .spawn(async move {
                    let dir = dirs::download_dir()
                        .filter(|d| d.is_dir())
                        .unwrap_or_else(std::env::temp_dir);
                    let mut saved = 0usize;
                    for att in &atts {
                        if let Some(data) = db.get_attachment_data(&att.id)? {
                            write_blob(&dir, &att.display_name, &data)?;
                            saved += 1;
                        }
                    }
                    Ok(saved)
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                match result {
                    Ok(n) => this.set_toast(&format!("Saved {n} attachment(s)."), cx),
                    Err(e) => this.set_toast(&format!("Download failed: {e}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Write the blob to a temp file and hand it to the OS default handler.
    /// This is also the safe path for PDFs (no in-app untrusted rendering).
    fn open_attachment(&mut self, att: Attachment, cx: &mut Context<Self>) {
        let db = self.db.clone();
        let entity = cx.entity();
        cx.spawn(async move |_weak, cx| {
            let result: anyhow::Result<()> = cx
                .background_executor()
                .spawn(async move {
                    let data = load_attachment_data(&db, &att)?;
                    let path = write_blob(&std::env::temp_dir(), &att.display_name, &data)?;
                    open::that(&path)?;
                    Ok(())
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                if let Err(e) = result {
                    this.set_toast(&format!("Could not open attachment: {e}"), cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Preview an image in-app, or open PDFs/other files in the OS viewer.
    fn preview_attachment(&mut self, att: Attachment, cx: &mut Context<Self>) {
        if is_image_name(&att.display_name) {
            let db = self.db.clone();
            let entity = cx.entity();
            cx.spawn(async move |_weak, cx| {
                let result: anyhow::Result<(String, PathBuf)> = cx
                    .background_executor()
                    .spawn(async move {
                        let data = load_attachment_data(&db, &att)?;
                        let path = write_blob(&std::env::temp_dir(), &att.display_name, &data)?;
                        Ok((att.display_name.clone(), path))
                    })
                    .await;
                let _ = entity.update(cx, move |this, cx| {
                    match result {
                        Ok((name, path)) => {
                            this.preview = Some(AttachmentPreview { name, path });
                        }
                        Err(e) => this.set_toast(&format!("Preview failed: {e}"), cx),
                    }
                    cx.notify();
                });
            })
            .detach();
        } else {
            if is_pdf_name(&att.display_name) {
                self.set_toast("PDF opens in your system viewer.", cx);
            }
            self.open_attachment(att, cx);
        }
    }

    fn close_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(preview) = self.preview.take() {
            let _ = std::fs::remove_file(&preview.path);
        }
        cx.notify();
    }

    fn preview_section(&self, cx: &mut Context<Self>, entity: Entity<Self>) -> Option<AnyElement> {
        let preview = self.preview.as_ref()?;
        let name = preview.name.clone();
        let path = preview.path.clone();
        Some(
            v_flex()
                .id("attachment-preview")
                .gap_2()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().muted)
                .p_2()
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_sm()
                                .font_medium()
                                .child(SharedString::from(name)),
                        )
                        .child(
                            Button::new("close-preview")
                                .icon(IconName::Close)
                                .ghost()
                                .xsmall()
                                .tooltip("Close preview")
                                .on_click({
                                    let entity = entity.clone();
                                    move |_, _, cx| {
                                        cx.update_entity(&entity, |this, cx| {
                                            this.close_preview(cx);
                                        });
                                    }
                                }),
                        ),
                )
                .child(
                    gpui::img(path)
                        .object_fit(gpui::ObjectFit::ScaleDown)
                        .w_full()
                        .max_h(px(320.0))
                        .rounded_md()
                        .bg(cx.theme().background),
                )
                .into_any_element(),
        )
    }

    fn attachments_section(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        message_id: &str,
    ) -> AnyElement {
        let atts = self.db.list_attachments(message_id).unwrap_or_default();
        if atts.is_empty() {
            return div().into_any_element();
        }
        let all = atts.clone();
        v_flex()
            .id("attachments")
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_medium()
                            .child(SharedString::from(format!(
                                "{} {}",
                                atts.len(),
                                if atts.len() == 1 {
                                    "Attachment"
                                } else {
                                    "Attachments"
                                }
                            ))),
                    )
                    .child(
                        h_flex().gap_1().child(
                            Button::new("download-all")
                                .icon(IconName::ArrowDown)
                                .ghost()
                                .small()
                                .tooltip("Download all")
                                .on_click({
                                    let entity = entity.clone();
                                    let all = all.clone();
                                    move |_, _, cx| {
                                        let all = all.clone();
                                        cx.update_entity(&entity, move |this, cx| {
                                            this.download_all_attachments(all, cx);
                                        });
                                    }
                                }),
                        ),
                    ),
            )
            .child(
                h_flex().gap_2().flex_wrap().children(
                    atts.iter()
                        .map(|att| self.attachment_card(cx, entity.clone(), att))
                        .collect::<Vec<_>>(),
                ),
            )
            .into_any_element()
    }

    fn attachment_card(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        att: &Attachment,
    ) -> AnyElement {
        let (color, short) = file_type_style(&att.display_name);
        let fg = cx.theme().muted_foreground;
        let preview_label = if is_image_name(&att.display_name) {
            "Preview image"
        } else {
            "Preview in system viewer"
        };
        h_flex()
            .gap_2()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .items_center()
            .child(
                div()
                    .w(px(36.0))
                    .h(px(36.0))
                    .rounded_md()
                    .bg(color)
                    .text_color(komorebi_hex(0xffffff))
                    .items_center()
                    .justify_center()
                    .flex_none()
                    .child(
                        div()
                            .text_xs()
                            .font_semibold()
                            .child(SharedString::from(short)),
                    ),
            )
            .child(
                v_flex()
                    .gap_0p5()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .text_sm()
                            .truncate()
                            .child(SharedString::from(att.display_name.clone())),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(fg)
                            .child(SharedString::from(format_bytes(att.size_bytes))),
                    ),
            )
            .child(
                h_flex()
                    .gap_0()
                    .child(
                        Button::new(SharedString::from(format!("preview-{}", att.id)))
                            .icon(IconName::Eye)
                            .ghost()
                            .xsmall()
                            .tooltip(preview_label)
                            .on_click({
                                let entity = entity.clone();
                                let att = att.clone();
                                move |_, _, cx| {
                                    let att = att.clone();
                                    cx.update_entity(&entity, move |this, cx| {
                                        this.preview_attachment(att, cx);
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("download-{}", att.id)))
                            .icon(IconName::ArrowDown)
                            .ghost()
                            .xsmall()
                            .tooltip("Download")
                            .on_click({
                                let entity = entity.clone();
                                let att = att.clone();
                                move |_, _, cx| {
                                    let att = att.clone();
                                    cx.update_entity(&entity, move |this, cx| {
                                        this.download_attachment(att, cx);
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("open-{}", att.id)))
                            .icon(IconName::ExternalLink)
                            .ghost()
                            .xsmall()
                            .tooltip("Open in system viewer")
                            .on_click({
                                let entity = entity.clone();
                                let att = att.clone();
                                move |_, _, cx| {
                                    let att = att.clone();
                                    cx.update_entity(&entity, move |this, cx| {
                                        this.open_attachment(att, cx);
                                    });
                                }
                            }),
                    ),
            )
            .into_any_element()
    }

    // ---------------------------------------------------------------------
    // AI panel
    // ---------------------------------------------------------------------
    fn ai_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let entity = cx.entity();
        let (name, email) = self.user_profile();
        let first_name = name
            .split_whitespace()
            .next()
            .unwrap_or("there")
            .to_string();
        let busy = self.ai_busy;
        let output = self.ai_output.clone();
        let error = self.ai_error.clone();
        let tone = self.ai_tone.clone();

        let topic = self
            .selected_message
            .as_ref()
            .and_then(|m| m.ai_category.clone())
            .unwrap_or_else(|| "General".to_string());
        let last_response = self
            .selected_message
            .as_ref()
            .and_then(|m| m.date_received)
            .map(format_relative_long)
            .unwrap_or_else(|| " - ".to_string());
        let sender_name = self
            .selected_message
            .as_ref()
            .and_then(|m| m.sender_name.clone())
            .unwrap_or_else(|| "Sender".to_string());
        let topic_badge = label_badge(cx, &topic);

        v_flex()
            .id("ai-panel")
            .h_full()
            .w_full()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_l_1()
            .border_color(cx.theme().sidebar_border)
            .overflow_y_scroll()
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .justify_between()
                    .items_center()
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                Icon::new(IconName::Bot)
                                    .size_4()
                                    .text_color(cx.theme().primary),
                            )
                            .child(Label::new("Komorebi AI").font_semibold())
                            .child(
                                div()
                                    .text_xs()
                                    .px_1()
                                    .rounded_sm()
                                    .bg(cx.theme().primary)
                                    .text_color(cx.theme().primary_foreground)
                                    .child("BETA"),
                            ),
                    )
                    .child(
                        Button::new("close-ai")
                            .icon(IconName::Close)
                            .ghost()
                            .xsmall()
                            .tooltip("Collapse AI panel")
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    cx.update_entity(&entity, |this, cx| {
                                        this.ai_panel_visible = false;
                                        cx.notify();
                                    });
                                }
                            }),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .pb_2()
                    .gap_2()
                    .items_center()
                    .child(Avatar::new().name(SharedString::from(name.clone())).small())
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_medium()
                                    .child(SharedString::from(name)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(SharedString::from(email)),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .size_3p5()
                            .text_color(cx.theme().muted_foreground),
                    ),
            )
            .child(
                v_flex()
                    .px_3()
                    .pb_2()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_semibold()
                            .child(SharedString::from(format!("Hi {first_name} 👋"))),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("How can I help with this email?"),
                    ),
            )
            .child(
                v_flex()
                    .px_2()
                    .gap_1()
                    .child(self.ai_action_card(
                        cx,
                        entity.clone(),
                        IconName::BookOpen,
                        "Summarize thread",
                        "Get a concise summary of this conversation",
                        AiKind::Summarize,
                    ))
                    .child(self.ai_action_card(
                        cx,
                        entity.clone(),
                        IconName::ArrowLeft,
                        "Draft a reply",
                        "Create a smart reply in your tone",
                        AiKind::DraftReply,
                    ))
                    .child(self.ai_action_card(
                        cx,
                        entity.clone(),
                        IconName::ALargeSmall,
                        "Improve writing",
                        "Enhance clarity, tone, and grammar",
                        AiKind::Polish,
                    ))
                    .child(self.ai_action_card(
                        cx,
                        entity.clone(),
                        IconName::CircleCheck,
                        "Detect action items",
                        "Identify tasks and next steps",
                        AiKind::Triage,
                    ))
                    .child(self.ai_action_card(
                        cx,
                        entity.clone(),
                        IconName::Folder,
                        "Categorize email",
                        "Suggest the best label",
                        AiKind::Triage,
                    ))
                    .child(self.ai_action_card(
                        cx,
                        entity.clone(),
                        IconName::Calendar,
                        "Schedule follow-up",
                        "Set a reminder to follow up",
                        AiKind::FollowUp,
                    )),
            )
            .child(self.section_header(cx, "AI Insights"))
            .child(
                v_flex()
                    .px_2()
                    .pb_2()
                    .gap_2()
                    .child(insight_row(cx, "This thread is about:", Some(topic_badge)))
                    .child(
                        h_flex()
                            .w_full()
                            .px_2()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Key people"),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .child(
                                        Avatar::new()
                                            .name(SharedString::from(sender_name.clone()))
                                            .small(),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .px_1p5()
                                            .rounded_full()
                                            .bg(cx.theme().muted)
                                            .text_color(cx.theme().muted_foreground)
                                            .child("+2"),
                                    ),
                            ),
                    )
                    .child(insight_row(
                        cx,
                        "Last response time",
                        Some(
                            div()
                                .text_sm()
                                .font_medium()
                                .child(SharedString::from(last_response))
                                .into_any_element(),
                        ),
                    ))
                    .child(
                        v_flex()
                            .px_2()
                            .gap_1()
                            .child(
                                h_flex()
                                    .w_full()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Your response rate"),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .items_center()
                                            .child(div().text_sm().font_medium().child("90%"))
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().success)
                                                    .child("▲ 12%"),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .h(px(4.0))
                                    .rounded_full()
                                    .bg(cx.theme().border)
                                    .child(
                                        div()
                                            .h_full()
                                            .w(px(150.0))
                                            .rounded_full()
                                            .bg(cx.theme().success),
                                    ),
                            ),
                    ),
            )
            .children(output.map(|o| {
                v_flex()
                    .id("ai-output")
                    .mx_2()
                    .mb_2()
                    .p_3()
                    .rounded_md()
                    .bg(cx.theme().muted)
                    .gap_1()
                    .child(
                        h_flex()
                            .justify_between()
                            .child(Label::new("Result").text_xs().font_semibold())
                            .child(if busy {
                                Icon::new(IconName::LoaderCircle)
                                    .size_3p5()
                                    .text_color(cx.theme().muted_foreground)
                                    .into_any_element()
                            } else {
                                div().into_any_element()
                            }),
                    )
                    .child(SharedString::from(o))
            }))
            .children(error.map(|e| {
                div()
                    .id("ai-error")
                    .mx_2()
                    .mb_2()
                    .p_2()
                    .rounded_md()
                    .text_color(cx.theme().danger)
                    .child(SharedString::from(e))
            }))
            .child(div().flex_1())
            .child(self.tone_footer(cx, entity, &tone))
    }

    fn ai_action_card(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        icon: IconName,
        title: &'static str,
        subtitle: &'static str,
        kind: AiKind,
    ) -> AnyElement {
        h_flex()
            .id(SharedString::from(format!("ai-card-{title}")))
            .w_full()
            .px_2()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            .gap_2()
            .items_center()
            .hover(|s| s.bg(cx.theme().muted))
            .child(Icon::new(icon).size_4().text_color(cx.theme().primary))
            .child(
                v_flex()
                    .flex_1()
                    .gap_0p5()
                    .child(div().text_sm().font_medium().child(title))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(subtitle),
                    ),
            )
            .child(
                Icon::new(IconName::ChevronRight)
                    .size_3p5()
                    .text_color(cx.theme().muted_foreground),
            )
            .on_click(move |_, window, cx| {
                cx.update_entity(&entity, move |this, cx| match kind {
                    AiKind::Summarize => this.run_ai(cx, AiKind::Summarize),
                    AiKind::DraftReply => this.run_ai_reply(window, cx),
                    AiKind::Polish => this.run_ai(cx, AiKind::Polish),
                    AiKind::FollowUp => this.schedule_followup(cx),
                    AiKind::Triage => {
                        let _ = title;
                        if title == "Detect action items" {
                            this.run_detect_actions(cx);
                        } else {
                            this.run_ai(cx, AiKind::Triage);
                        }
                    }
                });
            })
            .into_any_element()
    }

    fn tone_footer(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        tone: &str,
    ) -> impl IntoElement + use<> {
        h_flex()
            .id("tone-footer")
            .w_full()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(IconName::Palette)
                            .size_4()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("AI tone"),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Button::new("tone-select")
                            .label(SharedString::from(tone.to_string()))
                            .icon(IconName::ChevronDown)
                            .ghost()
                            .small()
                            .dropdown_menu({
                                let entity = entity.clone();
                                move |menu, _, _| {
                                    let tones = [
                                        "Professional",
                                        "Warm",
                                        "Concise",
                                        "Direct",
                                        "Friendly",
                                        "Formal",
                                    ];
                                    let mut m = menu;
                                    for t in tones {
                                        m = m.item(
                                            PopupMenuItem::new(SharedString::from(t)).on_click({
                                                let entity = entity.clone();
                                                move |_, _, cx| {
                                                    let t = t.to_string();
                                                    cx.update_entity(&entity, move |this, cx| {
                                                        let _ = this.db.set_setting("ai.tone", &t);
                                                        this.ai_tone = t;
                                                        cx.notify();
                                                    });
                                                }
                                            }),
                                        );
                                    }
                                    m
                                }
                            }),
                    )
                    .child(
                        Button::new("ai-settings")
                            .icon(IconName::Settings)
                            .ghost()
                            .xsmall()
                            .tooltip("AI settings")
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    cx.update_entity(&entity, |this, cx| {
                                        this.panel = Panel::Accounts;
                                        cx.notify();
                                    });
                                }
                            }),
                    ),
            )
    }

    // ---------------------------------------------------------------------
    // AI execution
    // ---------------------------------------------------------------------
    fn run_ai(&mut self, cx: &mut Context<Self>, kind: AiKind) {
        let Some(msg) = self.selected_message.clone() else {
            return;
        };
        let sender = msg.sender_name.clone();
        let subject = msg.subject.clone();
        // Summarize covers the whole conversation; other actions use the
        // selected message body only.
        let body = if matches!(kind, AiKind::Summarize) {
            format_thread_text(&self.thread_messages)
        } else {
            msg.body_text.clone().unwrap_or_default()
        };
        let message_id = msg.id.clone();
        let account_id = msg.account_id.clone();
        let ai = self.ai.clone();
        let db = self.db.clone();
        let tone = self.ai_tone.clone();
        let entity = cx.entity();

        self.ai_busy = true;
        self.ai_output = None;
        self.ai_error = None;
        cx.notify();

        cx.spawn(async move |_weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    perform_ai(
                        &ai,
                        &db,
                        kind,
                        &account_id,
                        &message_id,
                        sender.as_deref(),
                        subject.as_deref(),
                        &body,
                        &tone,
                    )
                })
                .await;
            let (output, error) = match result {
                Ok(text) => (Some(text), None),
                Err(e) => (None, Some(e.to_string())),
            };
            let _ = entity.update(cx, move |this, cx| {
                this.ai_busy = false;
                this.ai_output = output;
                this.ai_error = error;
                cx.notify();
            });
        })
        .detach();
    }

    fn run_detect_actions(&mut self, cx: &mut Context<Self>) {
        let Some(msg) = self.selected_message.clone() else {
            return;
        };
        let body = msg.body_text.clone().unwrap_or_default();
        self.ai_output = Some(detect_action_items(&body));
        self.ai_error = None;
        cx.notify();
    }

    // ---------------------------------------------------------------------
    // Compose
    // ---------------------------------------------------------------------
    fn compose_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let entity = cx.entity();
        let status = self.compose_status.clone();
        let to_input = self.to_input.clone();
        let subject_input = self.subject_input.clone();
        let body_input = self.body_input.clone();
        let attachments = self.composer_attachments.clone();
        let drag_over = self.composer_drag_over;

        v_flex()
            .id("compose")
            .flex_1()
            .h_full()
            .p_4()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .child(div().text_xl().font_semibold().child("New message"))
                    .child(
                        Button::new("close-compose")
                            .icon(IconName::Close)
                            .ghost()
                            .tooltip("Close")
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    cx.update_entity(&entity, |this, cx| {
                                        this.panel = Panel::Mail;
                                        cx.notify();
                                    });
                                }
                            }),
                    ),
            )
            .child(Input::new(&to_input))
            .child(Input::new(&subject_input))
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .min_h(px(240.0))
                    .rounded_md()
                    .border_1()
                    .border_color(if drag_over {
                        cx.theme().primary
                    } else {
                        cx.theme().border
                    })
                    .bg(cx.theme().background)
                    .when(drag_over, |d| d.bg(cx.theme().drop_target))
                    .child(Input::new(&body_input).border_0().h_full())
                    .on_drop::<ExternalPaths>({
                        let entity = entity.clone();
                        move |paths, _, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                this.add_attachments(paths, cx);
                            });
                        }
                    })
                    .on_drag_move::<ExternalPaths>({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                if !this.composer_drag_over {
                                    this.composer_drag_over = true;
                                    cx.notify();
                                }
                            });
                        }
                    }),
            )
            .child(self.attachment_row(cx, entity.clone(), attachments))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("send")
                            .label("Send")
                            .icon(IconName::ArrowRight)
                            .primary()
                            .on_click({
                                let entity = entity.clone();
                                move |_, window, cx| {
                                    cx.update_entity(&entity, |this, cx| {
                                        let _ = this.send_composed(window, cx);
                                    });
                                }
                            }),
                    )
                    .child(Button::new("cancel-compose").label("Discard").on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                this.panel = Panel::Mail;
                                this.compose_status = None;
                                this.composer_attachments.clear();
                                cx.notify();
                            });
                        }
                    })),
            )
            .children(status.map(|s| {
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(s))
            }))
    }

    fn attachment_row(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        attachments: Vec<PendingAttachment>,
    ) -> impl IntoElement + use<> {
        if attachments.is_empty() {
            return div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Drag files anywhere onto the message body to attach them.");
        }
        let mut chips = h_flex().gap_1().flex_wrap();
        for (ix, att) in attachments.into_iter().enumerate() {
            let name = att.name.clone();
            let size = att.size_bytes;
            chips = chips.child(
                h_flex()
                    .gap_1()
                    .px_2()
                    .py_0p5()
                    .rounded_md()
                    .bg(cx.theme().muted)
                    .text_sm()
                    .items_center()
                    .child(Icon::new(IconName::File).size_3p5())
                    .child(SharedString::from(format!(
                        "{name} ({})",
                        format_bytes(size)
                    )))
                    .child(
                        Button::new(SharedString::from(format!("detach-{ix}")))
                            .icon(IconName::Close)
                            .ghost()
                            .xsmall()
                            .tooltip("Remove attachment")
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    cx.update_entity(&entity, move |this, cx| {
                                        if ix < this.composer_attachments.len() {
                                            this.composer_attachments.remove(ix);
                                        }
                                        cx.notify();
                                    });
                                }
                            }),
                    ),
            );
        }
        chips
    }

    fn send_composed(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let to_raw = self.to_input.read(cx).value().to_string();
        let subject = self.subject_input.read(cx).value().to_string();
        let body = self.body_input.read(cx).value().to_string();

        let to: Vec<String> = to_raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();

        if to.is_empty() {
            self.compose_status = Some("Enter at least one recipient.".to_string());
            cx.notify();
            return false;
        }

        let attachments: Vec<mail::smtp::Attachment> = self
            .composer_attachments
            .iter()
            .map(|a| mail::smtp::Attachment {
                filename: a.name.clone(),
                content_type: mail::smtp::guess_content_type(&a.name).to_string(),
                data: a.data.clone(),
            })
            .collect();
        let account = self.accounts.first().cloned();

        // Clear the composer optimistically; the send owns its copies.
        self.to_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.subject_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.body_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.composer_attachments.clear();
        self.compose_status = Some("Sending…".to_string());
        cx.notify();

        let db = self.db.clone();
        let entity = cx.entity();
        cx.spawn(async move |_weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match account.as_ref() {
                        Some(a) => perform_send(&db, a, &to, &subject, &body, &attachments),
                        None => Ok(()),
                    }
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                this.compose_status = Some(match result {
                    Ok(()) => "Sent ✓".to_string(),
                    Err(e) => format!("Send failed: {e}"),
                });
                cx.notify();
            });
        })
        .detach();
        true
    }

    // ---------------------------------------------------------------------
    // Accounts
    // ---------------------------------------------------------------------
    /// Two-toggle TLS/STARTTLS picker for manual server settings.
    fn security_buttons(&self, entity: Entity<Self>, is_imap: bool) -> AnyElement {
        let current = if is_imap {
            self.setup_imap_security
        } else {
            self.setup_smtp_security
        };
        let mut row = h_flex().gap_1();
        for sec in [SetupSecurity::Tls, SetupSecurity::StartTls] {
            let selected = current == sec;
            let entity = entity.clone();
            row = row.child(
                Button::new(SharedString::from(format!(
                    "{}sec-{}",
                    if is_imap { "imap" } else { "smtp" },
                    sec.as_str()
                )))
                .label(SharedString::from(sec.label()))
                .when(selected, |b| b.primary())
                .when(!selected, |b| b.outline())
                .xsmall()
                .on_click(move |_, _, cx| {
                    cx.update_entity(&entity, move |this, cx| {
                        if is_imap {
                            this.setup_imap_security = sec;
                        } else {
                            this.setup_smtp_security = sec;
                        }
                        cx.notify();
                    });
                }),
            );
        }
        row.into_any_element()
    }

    /// Read-only summary of a successful autodiscovery result.
    fn discovered_summary(&self, cx: &mut Context<Self>, d: &DiscoveredSettings) -> AnyElement {
        let imap = d
            .imap
            .as_ref()
            .map(|s| format!("{}:{} ({})", s.host, s.port, s.security.as_str()))
            .unwrap_or_else(|| " - ".to_string());
        let smtp = d
            .smtp
            .as_ref()
            .map(|s| format!("{}:{} ({})", s.host, s.port, s.security.as_str()))
            .unwrap_or_else(|| " - ".to_string());
        v_flex()
            .gap_1()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .p_2()
            .child(
                div()
                    .text_sm()
                    .font_medium()
                    .child(SharedString::from(format!(
                        "{} · {}% confidence",
                        d.provider_label, d.confidence
                    ))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(format!("IMAP {imap} · SMTP {smtp}"))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(format!("Auth: {}", d.auth_mode))),
            )
            .children(d.warnings.iter().map(|w| {
                div()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child(SharedString::from(w.clone()))
            }))
            .into_any_element()
    }

    fn accounts_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let entity = cx.entity();
        let status = self.setup_status.clone();
        let accounts = self.accounts.clone();
        let setup_email = self.setup_email.clone();
        let setup_name = self.setup_name.clone();
        let setup_password = self.setup_password.clone();
        let setup_imap_host = self.setup_imap_host.clone();
        let setup_imap_port = self.setup_imap_port.clone();
        let setup_smtp_host = self.setup_smtp_host.clone();
        let setup_smtp_port = self.setup_smtp_port.clone();
        let setup_client_id = self.setup_client_id.clone();
        let setup_client_secret = self.setup_client_secret.clone();
        let setup_redirect_uri = self.setup_redirect_uri.clone();
        let discovered = self.setup_discovered.clone();
        let busy = self.setup_busy;
        let syncing = self.syncing;
        let sync_status = self.sync_status.clone();
        let ai_model = self.ai_model.clone();
        let ai_base_url = self.ai_base_url.clone();
        let current_provider = self.ai_config.provider.clone();
        let oauth = discovered
            .as_ref()
            .map(|d| d.auth_mode == "oauth")
            .unwrap_or(false);

        let add_or_authorize: AnyElement = if oauth {
            v_flex()
                .gap_2()
                .child(Label::new("OAuth client id").text_xs())
                .child(Input::new(&setup_client_id))
                .child(Label::new("OAuth client secret").text_xs())
                .child(Input::new(&setup_client_secret))
                .child(Label::new("Redirect URI").text_xs())
                .child(Input::new(&setup_redirect_uri))
                .child(
                    Button::new("authorize-oauth")
                        .label(if busy {
                            "Authorizing…"
                        } else {
                            "Authorize & add account"
                        })
                        .icon(IconName::Globe)
                        .primary()
                        .disabled(busy)
                        .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                                cx.update_entity(&entity, |this, cx| this.authorize_oauth(cx));
                            }
                        }),
                )
                .into_any_element()
        } else {
            Button::new("add-account")
                .label("Add account")
                .icon(IconName::Plus)
                .primary()
                .on_click({
                    let entity = entity.clone();
                    let email = setup_email.clone();
                    let name = setup_name.clone();
                    move |_, window, cx| {
                        let mut ok = false;
                        cx.update_entity(&entity, |this, cx| {
                            ok = this.add_account(cx);
                        });
                        if ok {
                            email.update(cx, |s, cx| s.set_value("", window, cx));
                            name.update(cx, |s, cx| s.set_value("", window, cx));
                        }
                    }
                })
                .into_any_element()
        };

        let mut providers = v_flex().id("ai-providers").gap_1();
        for preset in ai::providers::presets() {
            let id = preset.id.to_string();
            let label = preset.label.to_string();
            let selected = current_provider == id;
            providers = providers.child(
                Button::new(SharedString::from(format!("provider-{id}")))
                    .label(SharedString::from(label))
                    .when(selected, |b| b.primary())
                    .when(!selected, |b| b.outline())
                    .on_click({
                        let entity = entity.clone();
                        let model = ai_model.clone();
                        let base_url = ai_base_url.clone();
                        move |_, window, cx| {
                            let mut model_txt = String::new();
                            let mut base_txt = String::new();
                            let pid = id.clone();
                            cx.update_entity(&entity, |this, cx| {
                                if let Some(p) = ai::providers::preset(&pid) {
                                    this.ai_config = AiConfig::from_preset(&p);
                                    model_txt = p.default_model.to_string();
                                    base_txt = p.base_url.to_string();
                                }
                                cx.notify();
                            });
                            model.update(cx, |s, cx| {
                                s.set_value(SharedString::from(model_txt), window, cx);
                            });
                            base_url.update(cx, |s, cx| {
                                s.set_value(SharedString::from(base_txt), window, cx);
                            });
                        }
                    }),
            );
        }

        v_flex()
            .id("accounts")
            .flex_1()
            .h_full()
            .overflow_y_scroll()
            .p_4()
            .gap_2()
            .child(div().text_xl().font_semibold().child("Accounts"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "Self-hosted first: Mailcow, Zimbra, Poste.io, or any IMAP/SMTP server. Gmail and Microsoft 365 are supported through OAuth.",
                    ),
            )
            .child(div().text_lg().font_semibold().child("Add account"))
            .child(Input::new(&setup_email))
            .child(Input::new(&setup_name))
            .child(
                Button::new("discover")
                    .label(if busy { "Discovering…" } else { "Discover settings" })
                    .icon(IconName::Search)
                    .outline()
                    .disabled(busy)
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| this.discover_settings(cx));
                        }
                    }),
            )
            .children(discovered.as_ref().map(|d| self.discovered_summary(cx, d)))
            .child(Label::new("Password").text_xs())
            .child(Input::new(&setup_password))
            .child(
                Label::new("Manual server settings (used when discovery finds nothing)")
                    .text_xs(),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(Input::new(&setup_imap_host).flex_1())
                    .child(Input::new(&setup_imap_port).w(px(96.0))),
            )
            .child(self.security_buttons(entity.clone(), true))
            .child(
                h_flex()
                    .gap_2()
                    .child(Input::new(&setup_smtp_host).flex_1())
                    .child(Input::new(&setup_smtp_port).w(px(96.0))),
            )
            .child(self.security_buttons(entity.clone(), false))
            .child(add_or_authorize)
            .children(status.map(|s| {
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(s))
            }))
            .child(div().h_2())
            .child(div().text_lg().font_semibold().child("Sync"))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("sync")
                            .label(if syncing { "Syncing…" } else { "Sync all accounts" })
                            .icon(IconName::Globe)
                            .primary()
                            .disabled(syncing)
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    cx.update_entity(&entity, |this, cx| this.sync_now(cx));
                                }
                            }),
                    )
                    .children(sync_status.map(|s| {
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(SharedString::from(s))
                    })),
            )
            .child(self.account_rows(cx, entity.clone(), accounts))
            .child(div().h_4())
            .child(div().text_lg().font_semibold().child("AI provider"))
            .child(providers)
            .child(Label::new("Model").text_xs())
            .child(Input::new(&ai_model))
            .child(Label::new("Base URL").text_xs())
            .child(Input::new(&ai_base_url))
            .child(
                Button::new("save-ai")
                    .label("Save AI configuration")
                    .primary()
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            cx.update_entity(&entity, |this, cx| {
                                this.save_ai_config(cx);
                            });
                        }
                    }),
            )
    }

    fn account_rows(
        &self,
        cx: &mut Context<Self>,
        entity: Entity<Self>,
        accounts: Vec<Account>,
    ) -> impl IntoElement + use<> {
        let mut list = v_flex().gap_1();
        for account in accounts {
            let acc_id = account.id.clone();
            let email = account.email.clone();
            let provider = account.provider_kind.clone();
            let name = account
                .display_name
                .clone()
                .unwrap_or_else(|| email.clone());
            let row = h_flex()
                .w_full()
                .px_2()
                .py_2()
                .rounded_md()
                .border_1()
                .border_color(cx.theme().border)
                .justify_between()
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .child(Avatar::new().name(SharedString::from(name)).small())
                        .child(
                            v_flex().gap_0p5().child(SharedString::from(email)).child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(SharedString::from(provider)),
                            ),
                        ),
                )
                .child(
                    Button::new(SharedString::from(format!("del-{acc_id}")))
                        .label("Remove")
                        .ghost()
                        .danger()
                        .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                                let id = acc_id.clone();
                                cx.update_entity(&entity, move |this, cx| {
                                    let _ = this.db.delete_account(&id);
                                    this.refresh_accounts();
                                    this.refresh_folders();
                                    this.selected_message = None;
                                    this.select_first_folder();
                                    cx.notify();
                                });
                            }
                        }),
                );
            list = list.child(row);
        }
        let _ = cx;
        list
    }

    fn add_account(&mut self, cx: &mut Context<Self>) -> bool {
        let email = self.setup_email.read(cx).value().to_string();
        let name = self.setup_name.read(cx).value().to_string();
        if email.trim().is_empty() || !email.contains('@') {
            self.setup_status = Some("Enter a valid email address.".to_string());
            cx.notify();
            return false;
        }
        let email = email.trim().to_string();
        let display_name = if name.trim().is_empty() {
            None
        } else {
            Some(name.trim().to_string())
        };

        // Prefer discovered settings; otherwise read the manual fields.
        let discovered = self.setup_discovered.clone();
        let (kind, auth_mode, imap, smtp) = match &discovered {
            Some(d) => (
                ProviderKind::from_str(&d.provider_kind),
                d.auth_mode.clone(),
                d.imap.clone(),
                d.smtp.clone(),
            ),
            None => (
                ProviderKind::Custom,
                "password".to_string(),
                self.manual_imap(cx),
                self.manual_smtp(cx),
            ),
        };

        if auth_mode == "oauth" {
            self.setup_status = Some(
                "This account needs OAuth - fill in the client credentials and click Authorize."
                    .to_string(),
            );
            cx.notify();
            return false;
        }

        let password = self.setup_password.read(cx).value().to_string();
        if password.trim().is_empty() {
            self.setup_status = Some("Enter the mailbox password.".to_string());
            cx.notify();
            return false;
        }

        match mail::create_password_account(
            &self.db,
            &email,
            display_name.as_deref(),
            kind,
            imap.as_ref(),
            smtp.as_ref(),
            None,
            password.trim(),
        ) {
            Ok(acc) => {
                self.setup_status = Some(format!(
                    "Added {} - use Sync in the sidebar to fetch mail.",
                    acc.email
                ));
                self.setup_discovered = None;
                self.refresh_accounts();
                self.refresh_folders();
                cx.notify();
                true
            }
            Err(e) => {
                self.setup_status = Some(format!("Could not add account: {e}"));
                cx.notify();
                false
            }
        }
    }

    /// Read manual IMAP settings from the setup inputs (None if host is blank).
    fn manual_imap(&self, cx: &mut Context<Self>) -> Option<ServerSettings> {
        let host = self.setup_imap_host.read(cx).value().to_string();
        if host.trim().is_empty() {
            return None;
        }
        let port = self.setup_imap_port.read(cx).value().to_string();
        Some(ServerSettings {
            host: host.trim().to_string(),
            port: port.trim().parse().unwrap_or(993),
            security: self.setup_imap_security.security(),
        })
    }

    /// Read manual SMTP settings from the setup inputs (None if host is blank).
    fn manual_smtp(&self, cx: &mut Context<Self>) -> Option<ServerSettings> {
        let host = self.setup_smtp_host.read(cx).value().to_string();
        if host.trim().is_empty() {
            return None;
        }
        let port = self.setup_smtp_port.read(cx).value().to_string();
        Some(ServerSettings {
            host: host.trim().to_string(),
            port: port.trim().parse().unwrap_or(587),
            security: self.setup_smtp_security.security(),
        })
    }

    /// Run standards-based autodiscovery for the entered email address,
    /// off the UI thread, and store the result for the add/authorize buttons.
    fn discover_settings(&mut self, cx: &mut Context<Self>) {
        let email = self.setup_email.read(cx).value().to_string();
        if email.trim().is_empty() || !email.contains('@') {
            self.setup_status = Some("Enter a valid email address.".to_string());
            cx.notify();
            return;
        }
        let email = email.trim().to_string();
        self.setup_busy = true;
        self.setup_discovered = None;
        self.setup_status = Some("Discovering server settings…".to_string());
        cx.notify();

        let entity = cx.entity();
        cx.spawn(async move |_weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { mail::discover_account(&email) })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                this.setup_busy = false;
                match result {
                    Ok(d) => {
                        this.setup_status = Some(format!(
                            "Discovered via {} (confidence {}%) - review and add.",
                            d.source, d.confidence
                        ));
                        this.setup_discovered = Some(d);
                    }
                    Err(e) => {
                        this.setup_status = Some(format!(
                            "Discovery failed: {e} - enter server settings manually."
                        ));
                        this.setup_discovered = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Run the OAuth PKCE browser flow for a discovered Gmail/Microsoft account.
    fn authorize_oauth(&mut self, cx: &mut Context<Self>) {
        let email = self.setup_email.read(cx).value().to_string();
        let name = self.setup_name.read(cx).value().to_string();
        if email.trim().is_empty() || !email.contains('@') {
            self.setup_status = Some("Enter a valid email address.".to_string());
            cx.notify();
            return;
        }
        let Some(discovered) = self.setup_discovered.clone() else {
            self.setup_status =
                Some("Run discovery first to detect the OAuth provider.".to_string());
            cx.notify();
            return;
        };
        if discovered.auth_mode != "oauth" {
            self.setup_status = Some("This account uses password auth, not OAuth.".to_string());
            cx.notify();
            return;
        }

        let client_id = self.setup_client_id.read(cx).value().to_string();
        let client_secret = self.setup_client_secret.read(cx).value().to_string();
        let redirect = self.setup_redirect_uri.read(cx).value().to_string();
        if client_id.trim().is_empty() {
            self.setup_status = Some("Enter your OAuth client id.".to_string());
            cx.notify();
            return;
        }
        let redirect = if redirect.trim().is_empty() {
            "http://127.0.0.1:8765/callback".to_string()
        } else {
            redirect.trim().to_string()
        };
        let email = email.trim().to_string();
        let display_name = if name.trim().is_empty() {
            None
        } else {
            Some(name.trim().to_string())
        };
        let kind = ProviderKind::from_str(&discovered.provider_kind);
        let imap = discovered.imap.clone();
        let smtp = discovered.smtp.clone();

        self.setup_busy = true;
        self.setup_status = Some("Opening your browser for authorization…".to_string());
        cx.notify();

        let db = self.db.clone();
        let entity = cx.entity();
        let client_id = client_id.trim().to_string();
        let client_secret = client_secret.trim().to_string();
        cx.spawn(async move |_weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    mail::create_oauth_account(
                        &db,
                        &email,
                        display_name.as_deref(),
                        kind,
                        &client_id,
                        &client_secret,
                        &redirect,
                        imap.as_ref(),
                        smtp.as_ref(),
                        None,
                    )
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                this.setup_busy = false;
                match result {
                    Ok(acc) => {
                        this.setup_status = Some(format!(
                            "Authorized {} - use Sync to fetch mail.",
                            acc.email
                        ));
                        this.setup_discovered = None;
                        this.refresh_accounts();
                        this.refresh_folders();
                    }
                    Err(e) => this.setup_status = Some(format!("Authorization failed: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Sync every non-demo account off the UI thread, with visible status.
    fn sync_now(&mut self, cx: &mut Context<Self>) {
        if self.syncing {
            return;
        }
        let targets: Vec<Account> = self
            .accounts
            .iter()
            .filter(|a| a.provider_kind != "local")
            .cloned()
            .collect();
        if targets.is_empty() {
            self.set_toast(
                "Add a real account first (the demo account cannot sync).",
                cx,
            );
            return;
        }
        self.syncing = true;
        self.sync_status = Some("Syncing…".to_string());
        cx.notify();

        let db = self.db.clone();
        let entity = cx.entity();
        cx.spawn(async move |_weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut summary = mail::SyncSummary::default();
                    for account in &targets {
                        match mail::sync_account(&db, account) {
                            Ok(s) => {
                                summary.fetched += s.fetched;
                                summary.folders += s.folders;
                                summary.errors.extend(s.errors);
                            }
                            Err(e) => summary.errors.push(format!("{}: {e}", account.email)),
                        }
                    }
                    summary
                })
                .await;
            let _ = entity.update(cx, move |this, cx| {
                this.syncing = false;
                this.sync_status = Some(if result.errors.is_empty() {
                    format!(
                        "Synced {} message(s) across {} folder(s).",
                        result.fetched, result.folders
                    )
                } else {
                    format!(
                        "Synced {} message(s); {} error(s).",
                        result.fetched,
                        result.errors.len()
                    )
                });
                this.refresh_folders();
                this.reload_messages();
                cx.notify();
            });
        })
        .detach();
    }

    fn save_ai_config(&mut self, cx: &mut Context<Self>) {
        let model = self.ai_model.read(cx).value().to_string();
        let base_url = self.ai_base_url.read(cx).value().to_string();
        self.ai_config.model = model;
        self.ai_config.base_url = base_url;
        match ai::save_config(&self.db, &self.ai_config) {
            Ok(()) => {
                self.ai = Ai::load(&self.db).expect("saved config must load");
                self.ai_output = Some("Configuration saved.".to_string());
                self.ai_error = None;
            }
            Err(e) => {
                self.ai_error = Some(e.to_string());
                self.ai_output = None;
            }
        }
        cx.notify();
    }
}

impl Render for App {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar = self.sidebar(cx).into_any_element();
        let content: AnyElement = match self.panel {
            Panel::Mail => self.mail_panel(cx).into_any_element(),
            Panel::Compose => self.compose_panel(cx).into_any_element(),
            Panel::Accounts => self.accounts_panel(cx).into_any_element(),
        };
        let ai_sidebar = self.ai_sidebar(cx).into_any_element();
        let sidebar_visible = self.sidebar_visible;
        let ai_panel_visible = self.ai_panel_visible && self.panel == Panel::Mail;

        let mut body = h_resizable("main");
        if sidebar_visible {
            body = body.child(
                resizable_panel()
                    .size(px(248.0))
                    .size_range(px(200.0)..px(360.0))
                    .child(sidebar),
            );
        }
        body = body.child(resizable_panel().child(content));
        if ai_panel_visible {
            body = body.child(
                resizable_panel()
                    .size(px(300.0))
                    .size_range(px(240.0)..px(420.0))
                    .child(ai_sidebar),
            );
        }

        let entity = cx.entity();

        div()
            .id("app")
            .size_full()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &NewMessage, window, cx| {
                this.new_message(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.focus_search(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.toggle_sidebar(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleAiPanel, _, cx| {
                this.toggle_ai_panel(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleTheme, window, cx| {
                let next = if cx.theme().mode.is_dark() {
                    ThemeMode::Light
                } else {
                    ThemeMode::Dark
                };
                Theme::change(next, Some(window), cx);
                apply_komorebi_theme(window, cx);
                let _ = this;
            }))
            .on_action(cx.listener(|this, _: &NextMessage, _, cx| {
                this.select_relative(1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &PrevMessage, _, cx| {
                this.select_relative(-1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ArchiveMessage, _, cx| {
                this.archive_selected(cx);
            }))
            .on_action(cx.listener(|this, _: &StarMessage, _, cx| {
                this.toggle_selected_flag(cx);
            }))
            .on_action(cx.listener(|this, _: &ReplyMessage, window, cx| {
                this.open_reply(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ForwardMessage, window, cx| {
                this.open_forward(window, cx);
            }))
            .on_drag_move::<DragMessage>({
                let entity = entity.clone();
                move |_, _, cx| {
                    cx.update_entity(&entity, |this, cx| {
                        if this.drag_over_folder_id.is_some() {
                            this.drag_over_folder_id = None;
                            cx.notify();
                        }
                    });
                }
            })
            .on_drag_move::<ExternalPaths>({
                let entity = entity.clone();
                move |_, _, cx| {
                    cx.update_entity(&entity, |this, cx| {
                        if this.composer_drag_over {
                            this.composer_drag_over = false;
                            cx.notify();
                        }
                    });
                }
            })
            .child(self.titlebar(cx))
            .child(body)
    }
}

/// Send one message through the account's SMTP server, or record a local
/// sent copy for accounts without SMTP settings (the local demo account).
/// Runs off the UI thread.
fn perform_send(
    db: &Database,
    account: &Account,
    to: &[String],
    subject: &str,
    body: &str,
    attachments: &[mail::smtp::Attachment],
) -> anyhow::Result<()> {
    if account.smtp_host.is_some() {
        return mail::send_message(db, account, to, &[], subject, body, attachments);
    }
    // Local-only sent copy (no SMTP server configured).
    let sent = db
        .list_folders(&account.id)?
        .into_iter()
        .find(|f| f.special_use.as_deref() == Some("sent"))
        .or_else(|| db.list_folders(&account.id).ok()?.into_iter().next());
    if let Some(sent) = sent {
        let now = chrono::Utc::now().timestamp();
        let message_id = db.insert_message(&crate::db::MessageDraft {
            account_id: account.id.clone(),
            folder_id: sent.id.clone(),
            subject: Some(subject.to_string()),
            sender_email: Some(account.email.clone()),
            to_addresses: serde_json::to_string(to).ok(),
            date_received: Some(now),
            is_read: true,
            body_text: Some(body.to_string()),
            snippet: Some(mail::mime::make_snippet(body)),
            has_attachments: !attachments.is_empty(),
            ..Default::default()
        })?;
        for attachment in attachments {
            db.insert_attachment(
                &message_id,
                &attachment.filename,
                Some(&attachment.content_type),
                &attachment.data,
            )?;
        }
    }
    Ok(())
}

/// Run one AI action and persist the result to the cache.
#[allow(clippy::too_many_arguments)]
fn perform_ai(
    ai: &Ai,
    db: &Database,
    kind: AiKind,
    account_id: &str,
    message_id: &str,
    sender: Option<&str>,
    subject: Option<&str>,
    body: &str,
    tone: &str,
) -> anyhow::Result<String> {
    let (output, category) = match kind {
        AiKind::Summarize => {
            // `body` is the pre-formatted conversation text for summarize.
            let text = ai.summarize_thread(body)?;
            (text, None)
        }
        AiKind::DraftReply => {
            let text = ai.draft_reply(sender, subject, body, tone)?;
            (text, None)
        }
        AiKind::Polish => {
            let text = ai.polish(body, tone)?;
            (text, None)
        }
        AiKind::Triage => {
            let word = ai.triage(sender, subject, body)?;
            let category = Category::parse(&word).unwrap_or(Category::Updates);
            (format!("Category: {}", category.label()), Some(category))
        }
        AiKind::FollowUp => anyhow::bail!("follow-up is scheduled locally, not via an AI call"),
    };

    ai::record_and_apply(
        db,
        Some(account_id),
        Some(message_id),
        kind.as_str(),
        ai.config(),
        &output,
    )?;
    if let Some(category) = category {
        ai::apply_triage(db, message_id, category)?;
    } else if matches!(kind, AiKind::Summarize) {
        ai::apply_summary(db, message_id, &output)?;
    }
    Ok(output)
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

/// A small colored pill shown on message rows and in AI insights.
fn label_badge(cx: &mut Context<App>, label: &str) -> AnyElement {
    let color = label_color(label);
    h_flex()
        .flex_none()
        .gap_1()
        .px_1p5()
        .py_0p5()
        .rounded_full()
        .bg(cx.theme().muted)
        .items_center()
        .child(
            div()
                .w(px(6.0))
                .h(px(6.0))
                .rounded_full()
                .bg(color)
                .flex_none(),
        )
        .child(
            div()
                .text_xs()
                .text_color(color)
                .child(SharedString::from(label.to_string())),
        )
        .into_any_element()
}

fn label_color(label: &str) -> Hsla {
    let hex = match label.to_lowercase().as_str() {
        "work" => 0x6b9bd6,
        "design" => 0xa58bd6,
        "personal" => 0xa8c256,
        "finance" => 0xd68b4e,
        "travel" => 0xd66b5e,
        "clients" => 0x6b9bd6,
        "product" => 0x7e9a6b,
        "follow up" | "follow-up" => 0xc99a4a,
        _ => 0x9a9284,
    };
    komorebi_hex(hex)
}

/// Category + optional "Follow up" tag shown beside the reader subject.
fn category_tags(cx: &mut Context<App>, category: Option<&str>, flagged: bool) -> Vec<AnyElement> {
    let mut tags = Vec::new();
    if let Some(cat) = category {
        tags.push(label_badge(cx, cat));
    }
    if flagged {
        tags.push(label_badge(cx, "Follow up"));
    }
    tags
}

/// A calendar tag for an active snooze or follow-up reminder.
fn reminder_tag(cx: &mut Context<App>, label: &str) -> AnyElement {
    h_flex()
        .flex_none()
        .gap_1()
        .px_1p5()
        .py_0p5()
        .rounded_full()
        .bg(cx.theme().muted)
        .items_center()
        .child(
            Icon::new(IconName::Calendar)
                .size_3p5()
                .text_color(cx.theme().warning),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().warning)
                .child(SharedString::from(label.to_string())),
        )
        .into_any_element()
}

fn file_type_style(name: &str) -> (Hsla, &'static str) {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => (komorebi_hex(0xc9684e), "PDF"),
        "xlsx" | "xls" => (komorebi_hex(0x7e9a6b), "XLS"),
        "docx" | "doc" => (komorebi_hex(0x6b7fa0), "DOC"),
        "png" | "jpg" | "jpeg" | "gif" => (komorebi_hex(0xa58bd6), "IMG"),
        "zip" => (komorebi_hex(0xc99a4a), "ZIP"),
        _ => (komorebi_hex(0x9a9284), "FILE"),
    }
}

/// A single-line recipient hint: "to me" or "to {first recipient}".
fn recipient_line(to_json: &str, sender_email: &str) -> String {
    let recipients: Vec<String> = serde_json::from_str(to_json).unwrap_or_default();
    match recipients.first() {
        Some(r) if r == sender_email => "to me".to_string(),
        Some(r) => format!("to {r}"),
        None => "to me".to_string(),
    }
}

/// Format a whole conversation as per-message blocks for the AI summarize
/// action (sender + subject + body per message, oldest first).
fn format_thread_text(messages: &[MessageDetail]) -> String {
    messages
        .iter()
        .map(|m| {
            let sender = m
                .sender_name
                .clone()
                .unwrap_or_else(|| m.sender_email.clone().unwrap_or_default());
            let subject = m
                .subject
                .clone()
                .unwrap_or_else(|| "(no subject)".to_string());
            let body = m.body_text.clone().unwrap_or_default();
            format!("From: {sender}\nSubject: {subject}\n\n{}", body.trim())
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

/// Render plain text as line-separated blocks (the demo bodies use `\n\n`).
fn render_body(body: &str) -> AnyElement {
    let mut col = v_flex().gap_1p5();
    let mut first = true;
    for para in body.split("\n\n") {
        if !first {
            col = col.child(div().h_1p5());
        }
        first = false;
        for line in para.lines() {
            col = col.child(div().text_sm().child(SharedString::from(line.to_string())));
        }
    }
    col.into_any_element()
}

/// A no-op formatting button used in the inline composer toolbar.
fn format_button(label: &'static str, tooltip: &'static str) -> AnyElement {
    Button::new(SharedString::from(format!("fmt-{label}")))
        .label(SharedString::from(label))
        .ghost()
        .small()
        .tooltip(tooltip)
        .into_any_element()
}

fn insight_row(cx: &mut Context<App>, label: &str, value: Option<AnyElement>) -> AnyElement {
    h_flex()
        .w_full()
        .px_2()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(SharedString::from(label.to_string())),
        )
        .children(value)
        .into_any_element()
}

/// Heuristic local "action item" extraction (no network / model required).
fn detect_action_items(body: &str) -> String {
    let mut items: Vec<String> = Vec::new();
    for line in body.lines() {
        let t = line.trim();
        let bullet = t
            .strip_prefix('\u{2022}')
            .or_else(|| t.strip_prefix('-'))
            .map(str::trim)
            .unwrap_or(t);
        if (t.starts_with('\u{2022}') || t.starts_with('-') || (t.ends_with('?') && t.len() > 12))
            && !items.contains(&bullet.to_string())
        {
            items.push(bullet.to_string());
        }
    }
    if items.is_empty() {
        "No explicit action items detected in this message.".to_string()
    } else {
        format!(
            "Detected action items:\n{}",
            items
                .iter()
                .map(|i| format!("• {i}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

fn format_list_date(ts: Option<i64>) -> String {
    let Some(t) = ts else { return String::new() };
    let Some(dt) = chrono::DateTime::from_timestamp(t, 0) else {
        return String::new();
    };
    let diff = chrono::Utc::now().timestamp() - t;
    if diff < 60 {
        "now".into()
    } else if diff < 3600 {
        format!("{}m", diff / 60)
    } else if diff < 86400 {
        format!("{}h", diff / 3600)
    } else if diff < 7 * 86400 {
        dt.format("%a").to_string()
    } else {
        dt.format("%b %d").to_string()
    }
}

fn format_full_date(ts: Option<i64>) -> String {
    match ts {
        Some(t) => chrono::DateTime::from_timestamp(t, 0)
            .map(|d| d.format("%b %d, %Y · %H:%M").to_string())
            .unwrap_or_default(),
        None => String::new(),
    }
}

fn format_relative_long(ts: i64) -> String {
    let diff = chrono::Utc::now().timestamp() - ts;
    if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        let h = diff / 3600;
        let m = (diff % 3600) / 60;
        format!("{h}h {m:02}m")
    } else {
        format!("{}d ago", diff / 86400)
    }
}

fn format_bytes(bytes: i64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// A compact unread-count badge, or nothing when there are no unread messages.
fn unread_guard(unread: i64, cx: &mut Context<App>) -> Option<AnyElement> {
    if unread <= 0 {
        return None;
    }
    Some(
        div()
            .text_xs()
            .font_medium()
            .px_1p5()
            .rounded_full()
            .bg(cx.theme().sidebar_primary)
            .text_color(cx.theme().sidebar_primary_foreground)
            .child(SharedString::from(unread.to_string()))
            .into_any_element(),
    )
}

/// A small star marker shown for flagged messages in the list.
fn star_marker(flagged: bool, cx: &mut Context<App>) -> Option<AnyElement> {
    if !flagged {
        return None;
    }
    Some(
        Icon::new(IconName::Star)
            .size_3p5()
            .text_color(cx.theme().primary)
            .into_any_element(),
    )
}

// ---------------------------------------------------------------------------
// Attachment file helpers
// ---------------------------------------------------------------------------

fn load_attachment_data(db: &Database, att: &Attachment) -> anyhow::Result<Vec<u8>> {
    db.get_attachment_data(&att.id)?
        .ok_or_else(|| anyhow::anyhow!("attachment data missing for {}", att.display_name))
}

/// Sanitize a filename so it can be safely written to disk (no path
/// separators, control characters, or Windows-invalid characters).
fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        "attachment".to_string()
    } else {
        cleaned
    }
}

/// Find a non-colliding path in `dir` for `name`, appending " (n)" when needed.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let name = sanitize_filename(name);
    let candidate = dir.join(&name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) => (stem.to_string(), format!(".{ext}")),
        None => (name.clone(), String::new()),
    };
    for i in 1.. {
        let candidate = dir.join(format!("{stem} ({i}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

/// Write raw bytes to `dir` under a safe, collision-free filename.
fn write_blob(dir: &Path, name: &str, data: &[u8]) -> anyhow::Result<PathBuf> {
    let path = unique_path(dir, name);
    std::fs::write(&path, data)?;
    Ok(path)
}

fn is_image_name(name: &str) -> bool {
    matches!(
        name.rsplit('.')
            .next()
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some(
            "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "webp"
                | "bmp"
                | "ico"
                | "svg"
                | "avif"
                | "tif"
                | "tiff"
        )
    )
}

fn is_pdf_name(name: &str) -> bool {
    name.rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
        == Some("pdf")
}

/// Open the on-disk cache, falling back to an in-memory database if the config
/// directory or file is unavailable.
fn open_database() -> Database {
    if let Some(dir) = dirs::config_dir() {
        let path = dir.join("komorebi").join("cache.db");
        if let Ok(db) = Database::open(&path) {
            return db;
        }
    }
    Database::in_memory().expect("in-memory database must open")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_filename_strips_dangerous_chars() {
        assert_eq!(sanitize_filename("plain.txt"), "plain.txt");
        assert_eq!(sanitize_filename("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize_filename("a<b>:c|d?e*"), "a_b__c_d_e_");
        assert_eq!(sanitize_filename("..."), "attachment");
    }

    #[test]
    fn unique_path_appends_suffix_on_collision() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), b"x").unwrap();
        let p = unique_path(dir.path(), "note.txt");
        assert_eq!(p.file_name().unwrap().to_str().unwrap(), "note (1).txt");
        assert!(!p.exists());
    }

    #[test]
    fn file_kind_detection() {
        assert!(is_image_name("photo.PNG"));
        assert!(is_image_name("gif.gif"));
        assert!(is_pdf_name("invoice.pdf"));
        assert!(!is_pdf_name("photo.png"));
        assert!(!is_image_name("notes.txt"));
    }
}
