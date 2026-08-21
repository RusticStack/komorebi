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
use crate::db::{
    Account, Attachment, Database, Folder, Label as DbLabel, MessageDetail, MessageRow, Reminder,
};
use crate::i18n;
use crate::mail;
use crate::mail::discovery::DiscoveredSettings;
use crate::mail::types::{ProviderKind, Security, ServerSettings};
use crate::notify;
use gpui::prelude::*;
use gpui::{
    AnyElement, ClipboardItem, Context, Entity, ExternalPaths, FocusHandle, Focusable, KeyBinding,
    Pixels, Render, SharedString, StatefulInteractiveElement, Subscription, Window, actions, div,
    px, uniform_list,
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
use std::path::PathBuf;

// Shared view-state types live in `types.rs`.
mod types;
use types::*;

// Panel modules: each holds an `impl App` block for its surface.
pub mod accounts;
mod actions;
mod ai_panel;
pub mod attachments;
mod composer;
mod drafts_ui;
mod menus;
mod message_list;
mod reader;
mod send;
mod settings;
mod sidebar;
mod titlebar;

// Design tokens and theme live in `theme.rs`.
pub mod theme;
use theme::{RADIUS_LG, RADIUS_SM, apply_komorebi_theme, komorebi_hex};

// Free rendering helpers live in `helpers.rs`.
pub mod helpers;
use helpers::{
    category_tags, file_type_style, format_bytes, format_full_date, format_list_date,
    format_relative_long, format_thread_text, insight_row, is_image_name, is_pdf_name, label_badge,
    load_attachment_data, open_database, recipient_line, reminder_tag, render_message_body,
    star_marker, stored_label_color, unread_guard, write_blob,
};

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
    cc_input: Entity<InputState>,
    bcc_input: Entity<InputState>,
    compose_cc_bcc_open: bool,
    subject_input: Entity<InputState>,
    body_input: Entity<InputState>,
    compose_status: Option<String>,
    composer_attachments: Vec<PendingAttachment>,
    composer_drag_over: bool,

    // Per-account signature editor (Accounts panel).
    signature_input: Entity<InputState>,
    signature_account_id: Option<String>,

    // User labels (data-driven sidebar).
    labels: Vec<DbLabel>,
    labels_adding: bool,
    renaming_label: Option<String>,
    label_input: Entity<InputState>,

    // Draft autosave state.
    draft_id: Option<String>,
    draft_dirty: bool,

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
    ai_health: Option<String>,
    ai_health_busy: bool,

    // Locale (i18n).
    locale: i18n::Locale,

    // Search + navigation.
    search_input: Entity<InputState>,
    search_query: String,
    _subscriptions: Vec<Subscription>,
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

    // Remote-content consent for the current message (privacy-first: off by default).
    allow_remote_content: bool,
    remote_content_banner: Option<bool>,

    // Desktop notifications config.
    notify_config: notify::NotificationConfig,

    // Inline reply composer.
    reply_input: Entity<InputState>,
    reply_open: bool,
    reply_mode: ReplyMode,
    reply_to: String,
    reply_cc: String,
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
        // First-run onboarding: real users start with an empty mailbox and are
        // routed to account setup; the demo mailbox is opt-in from there.

        let ai = Ai::load(&db).unwrap_or_else(|_| Ai::load(&db).expect("ollama default must load"));
        let ai_config = ai.config().clone();

        // Persisted AI tone preference (falls back to Professional).
        let ai_tone = db
            .get_setting("ai.tone")
            .ok()
            .flatten()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "Professional".to_string());

        let locale = db
            .get_setting("ui.locale")
            .ok()
            .flatten()
            .map(|s| {
                let loc = i18n::Locale::from_str(&s);
                i18n::set_locale(loc);
                loc
            })
            .unwrap_or_else(|| {
                let loc = i18n::detect_locale();
                i18n::set_locale(loc);
                loc
            });

        let notify_config = notify::NotificationConfig {
            enabled: db
                .get_setting("notify.enabled")
                .ok()
                .flatten()
                .map(|v| v == "1")
                .unwrap_or(false),
            ..Default::default()
        };

        let to_input = cx.new(|cx| InputState::new(window, cx).placeholder("To (comma separated)"));
        let cc_input = cx.new(|cx| InputState::new(window, cx).placeholder("Cc (comma separated)"));
        let bcc_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Bcc (comma separated)"));
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
        let signature_input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("Write your signature…")
        });
        let label_input = cx.new(|cx| InputState::new(window, cx).placeholder("New label"));
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
        let search_subscription = cx.subscribe(&search_input, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.search_query = this.search_input.read(cx).value().to_string();
                this.reload_messages();
                cx.notify();
            }
        });

        // Draft autosave: any compose-field edit schedules a debounced flush.
        let mut subscriptions = vec![search_subscription];
        for input in [&to_input, &subject_input, &body_input] {
            subscriptions.push(cx.subscribe(input, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.queue_draft_autosave(cx);
                }
            }));
        }

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
            cc_input,
            bcc_input,
            compose_cc_bcc_open: false,
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
            ai_health: None,
            ai_health_busy: false,
            locale,
            search_input,
            search_query: String::new(),
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
            allow_remote_content: false,
            remote_content_banner: None,
            notify_config,
            reply_input,
            reply_open: false,
            reply_mode: ReplyMode::Reply,
            reply_to: String::new(),
            reply_cc: String::new(),
            reply_subject: String::new(),
            signature_input,
            signature_account_id: None,
            labels: Vec::new(),
            labels_adding: false,
            renaming_label: None,
            label_input,
            draft_id: None,
            draft_dirty: false,
            _subscriptions: subscriptions,
        };
        this.refresh_accounts();
        this.refresh_folders();
        this.select_first_folder();
        this.signature_account_id = this.accounts.first().map(|a| a.id.clone());
        // First run with no accounts: land on the setup/onboarding panel.
        if this.accounts.is_empty() {
            this.panel = Panel::Accounts;
        }

        apply_komorebi_theme(window, cx);
        this
    }

    // ---------------------------------------------------------------------
    // Data helpers
    // ---------------------------------------------------------------------
    fn refresh_accounts(&mut self) {
        self.accounts = self.db.list_accounts().unwrap_or_default();
        self.refresh_labels();
    }

    /// Reload user labels for the active account.
    pub(crate) fn refresh_labels(&mut self) {
        self.labels = self
            .accounts
            .first()
            .map(|a| self.db.list_labels(&a.id).unwrap_or_default())
            .unwrap_or_default();
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
            Some(id) if id.starts_with(LABEL_PREFIX) => self
                .db
                .list_messages_by_label(&id[LABEL_PREFIX.len()..], 500)
                .unwrap_or_default(),
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
        // Reset per-message remote-content consent (privacy-first: off by default).
        self.allow_remote_content = false;
        self.remote_content_banner = None;
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

    /// Grant remote-content consent for the current message and re-render.
    fn allow_remote(&mut self, cx: &mut Context<Self>) {
        self.allow_remote_content = true;
        cx.notify();
    }

    /// The remote-content consent banner: shown when the current message has
    /// remote resources that are blocked. Privacy-first - never auto-loads.
    fn remote_content_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let html = self
            .selected_message
            .as_ref()
            .and_then(|m| m.body_html.as_deref())?;
        if html.trim().is_empty() {
            return None;
        }
        let urls = mail::html::remote_content_urls(html);
        if urls.is_empty() {
            return None;
        }
        if self.allow_remote_content {
            return None;
        }
        Some(
            h_flex()
                .w_full()
                .gap_2()
                .px_3()
                .py_2()
                .rounded(RADIUS_SM)
                .bg(cx.theme().warning.opacity(0.12))
                .border_1()
                .border_color(cx.theme().warning.opacity(0.3))
                .child(
                    Icon::new(IconName::TriangleAlert)
                        .size(px(16.0))
                        .text_color(cx.theme().warning),
                )
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(SharedString::from(format!(
                            "Remote content blocked - {} image(s)/link(s) from {} server(s).",
                            urls.len(),
                            urls.iter()
                                .filter_map(|u| url::Url::parse(u).ok())
                                .map(|u| u.host_str().unwrap_or("?").to_string())
                                .collect::<std::collections::HashSet<_>>()
                                .len()
                        ))),
                )
                .child(
                    Button::new(SharedString::from("allow-remote"))
                        .label(SharedString::from("Load remote content"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.allow_remote(cx);
                        })),
                )
                .into_any_element(),
        )
    }

    fn current_view_label(&self) -> String {
        match self.selected_folder_id.as_deref() {
            Some(PRIORITY) => "Priority".to_string(),
            Some(SNOOZED) => "Snoozed".to_string(),
            Some(VIEW_UNREAD) => "Unread".to_string(),
            Some(VIEW_ATTACHMENTS) => "Has attachments".to_string(),
            Some(VIEW_MENTIONS) => "Mentions".to_string(),
            Some(id) if id.starts_with(LABEL_PREFIX) => self
                .labels
                .iter()
                .find(|l| l.id == id[LABEL_PREFIX.len()..])
                .map(|l| l.name.clone())
                .unwrap_or_else(|| "Label".to_string()),
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
        self.resume_latest_draft(window, cx);
        if let Some(sig) = self.signature_for_account(None) {
            let existing = self.body_input.read(cx).value().to_string();
            let body = if existing.trim().is_empty() {
                sig
            } else {
                format!("{existing}\n\n{sig}")
            };
            self.body_input
                .update(cx, |s, cx| s.set_value(body, window, cx));
        }
        self.panel = Panel::Compose;
        cx.notify();
    }

    /// Prefill the composer from the most recent saved draft, if any.
    /// Opt-in demo mailbox for exploring the client before adding an account.
    pub(crate) fn seed_demo(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = mail::seed_local_demo(&self.db) {
            log::error!("failed to seed demo mailbox: {e}");
            self.set_toast("Could not load the demo mailbox.", cx);
            return;
        }
        self.refresh_accounts();
        self.refresh_folders();
        self.select_first_folder();
        self.signature_account_id = self.accounts.first().map(|a| a.id.clone());
        self.panel = Panel::Mail;
        self.set_toast("Demo mailbox loaded", cx);
    }

    /// The stored signature for `account_id` (or the first account).
    fn clear_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.compose_status = None;
        self.composer_attachments.clear();
        self.composer_drag_over = false;
        self.compose_cc_bcc_open = false;
        self.to_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.cc_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.bcc_input
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
        self.reply_cc = String::new();
        self.reply_subject = String::new();
        self.ai_reply_draft = None;
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
