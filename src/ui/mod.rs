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
mod ai_panel;
mod composer;
mod message_list;
mod reader;
mod sidebar;

// Design tokens and theme live in `theme.rs`.
pub mod theme;
use theme::{RADIUS_LG, RADIUS_SM, apply_komorebi_theme, komorebi_hex};

// Free rendering helpers live in `helpers.rs`.
pub mod helpers;
use helpers::{
    category_tags, file_type_style, format_bytes, format_full_date, format_list_date,
    format_relative_long, format_thread_text, insight_row, is_image_name, is_pdf_name, label_badge,
    load_attachment_data, open_database, recipient_line, reminder_tag, render_message_body,
    star_marker, unread_guard, write_blob,
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
    fn resume_latest_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(drafts) = self.db.list_drafts() else {
            return;
        };
        let Some(draft) = drafts.into_iter().max_by_key(|d| d.updated_at) else {
            return;
        };
        if draft.to_addresses.is_none() && draft.subject.is_none() && draft.body.is_none() {
            return;
        }
        self.draft_id = Some(draft.id);
        if let Some(to) = draft.to_addresses {
            self.to_input
                .update(cx, |s, cx| s.set_value(to, window, cx));
        }
        if let Some(subject) = draft.subject {
            self.subject_input
                .update(cx, |s, cx| s.set_value(subject, window, cx));
        }
        if let Some(body) = draft.body {
            self.body_input
                .update(cx, |s, cx| s.set_value(body, window, cx));
        }
    }

    /// Schedule a debounced autosave after a compose-field edit.
    fn queue_draft_autosave(&mut self, cx: &mut Context<Self>) {
        if self.panel != Panel::Compose {
            return;
        }
        self.draft_dirty = true;
        let entity = cx.entity();
        cx.spawn(async move |_this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(2))
                .await;
            let _ = entity.update(cx, |this, cx| this.flush_draft(cx));
        })
        .detach();
    }

    /// Write the current compose fields to the drafts table (no-op when
    /// nothing changed or everything is empty).
    pub(crate) fn flush_draft(&mut self, cx: &mut Context<Self>) {
        if !self.draft_dirty || self.panel != Panel::Compose {
            return;
        }
        let to = self.to_input.read(cx).value().trim().to_string();
        let subject = self.subject_input.read(cx).value().trim().to_string();
        let body = self.body_input.read(cx).value().to_string();
        if to.is_empty() && subject.is_empty() && body.trim().is_empty() {
            return;
        }
        let account_id = self
            .accounts
            .first()
            .map(|a| a.id.clone())
            .unwrap_or_default();
        let result = match self.draft_id.clone() {
            Some(id) => self
                .db
                .update_draft(&id, Some(&subject), Some(&to), Some(&body))
                .map(|_| id),
            None => self
                .db
                .save_draft(&account_id, Some(&subject), Some(&to), Some(&body)),
        };
        match result {
            Ok(id) => self.draft_id = Some(id),
            Err(e) => log::error!("draft autosave failed: {e}"),
        }
        self.draft_dirty = false;
        cx.notify();
    }

    /// Discard the draft currently being edited.
    pub(crate) fn discard_draft(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.draft_id.take() {
            let _ = self.db.delete_draft(&id);
        }
        self.draft_dirty = false;
        cx.notify();
    }

    /// Opt-in demo mailbox for exploring the client before adding an account.
    fn seed_demo(&mut self, cx: &mut Context<Self>) {
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
    fn signature_for_account(&self, account_id: Option<&str>) -> Option<String> {
        let id = account_id
            .map(str::to_string)
            .or_else(|| self.signature_account_id.clone());
        let account = match id {
            Some(id) => self
                .accounts
                .iter()
                .find(|a| a.id == id)
                .or_else(|| self.accounts.first()),
            None => self.accounts.first(),
        };
        account
            .and_then(|a| a.signature_html.clone())
            .filter(|s| !s.trim().is_empty())
    }

    /// Load an account's signature into the editor.
    fn load_signature(&mut self, account_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.signature_account_id = Some(account_id.to_string());
        let sig = self
            .accounts
            .iter()
            .find(|a| a.id == account_id)
            .and_then(|a| a.signature_html.clone())
            .unwrap_or_default();
        self.signature_input
            .update(cx, |s, cx| s.set_value(&sig, window, cx));
        cx.notify();
    }

    /// Persist the signature editor contents to the selected account.
    fn save_signature(&mut self, cx: &mut Context<Self>) {
        let Some(account_id) = self.signature_account_id.clone() else {
            self.set_toast("Add an account first.", cx);
            return;
        };
        let value = self.signature_input.read(cx).value().to_string();
        if let Err(e) = self.db.update_account_signature(&account_id, &value) {
            log::error!("failed to save signature: {e}");
            self.set_toast("Could not save signature.", cx);
            return;
        }
        self.refresh_accounts();
        self.set_toast("Signature saved ✓", cx);
    }

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
        let parse = |raw: Option<&String>| -> Vec<String> {
            raw.and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
                .unwrap_or_default()
        };
        let own = self.accounts.first().map(|a| a.email.clone());
        let mut to = vec![msg.sender_email.clone().unwrap_or_default()];
        to.extend(parse(msg.to_addresses.as_ref()));
        let mut cc = parse(msg.cc_addresses.as_ref());
        // Reply-all etiquette: everyone originally addressed goes back on
        // To/CC respectively, minus our own address and duplicates.
        let mut seen: Vec<String> = Vec::new();
        let dedup = |list: &mut Vec<String>, seen: &mut Vec<String>| {
            list.retain(|addr| {
                let addr = addr.trim();
                if addr.is_empty() || own.as_deref() == Some(addr) || seen.iter().any(|s| s == addr)
                {
                    return false;
                }
                seen.push(addr.to_string());
                true
            });
        };
        dedup(&mut to, &mut seen);
        dedup(&mut cc, &mut seen);
        let subject = format!(
            "Re: {}",
            msg.subject.clone().unwrap_or_else(|| "(no subject)".into())
        );
        self.clear_reply(window, cx);
        self.reply_open = true;
        self.reply_mode = ReplyMode::Reply;
        self.reply_to = to.join(", ");
        self.reply_cc = cc.join(", ");
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
        let cc: Vec<String> = self
            .reply_cc
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
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
        self.reply_cc = String::new();
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
                        Some(a) => {
                            perform_send(&db, a, &to, &cc, &[], &subject, &body, &attachments)
                        }
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
        let sidebar_visible = self.sidebar_visible;

        TitleBar::new().child(
            h_flex()
                .w_full()
                .h_full()
                .items_center()
                .gap_2()
                // Sidebar toggle in the title bar - consistent, discoverable navigation.
                .child(
                    Button::new("title-sidebar-toggle")
                        .icon(if sidebar_visible {
                            IconName::PanelLeftClose
                        } else {
                            IconName::PanelLeftOpen
                        })
                        .ghost()
                        .xsmall()
                        .tooltip(if sidebar_visible {
                            "Hide sidebar (Ctrl+B)"
                        } else {
                            "Show sidebar (Ctrl+B)"
                        })
                        .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                                cx.update_entity(&entity, |this, cx| {
                                    this.toggle_sidebar(cx);
                                });
                            }
                        }),
                )
                .child(
                    div()
                        .w(px(22.0))
                        .h(px(22.0))
                        .rounded_full()
                        // Japandi: the brand mark is understated - a soft
                        // ring with the sun glyph, not a bold filled disc.
                        .border_1()
                        .border_color(cx.theme().primary)
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(IconName::Sun)
                                .size_3()
                                .text_color(cx.theme().primary),
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
#[allow(clippy::too_many_arguments)]
fn perform_send(
    db: &Database,
    account: &Account,
    to: &[String],
    cc: &[String],
    bcc: &[String],
    subject: &str,
    body: &str,
    attachments: &[mail::smtp::Attachment],
) -> anyhow::Result<()> {
    if account.smtp_host.is_some() {
        return mail::send_message(db, account, to, cc, subject, body, attachments);
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
            cc_addresses: serde_json::to_string(cc).ok(),
            bcc_addresses: serde_json::to_string(bcc).ok(),
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
