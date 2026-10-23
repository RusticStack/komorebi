//! Settings beyond the account form: the General choices (theme, check
//! interval, undo-send and mark-as-read delays, taskbar flash), the
//! diagnostics log switch (Privacy), account
//! housekeeping (Enabled, order, Re-download), the window title's unread
//! count and the window's place between runs.
//!
//! A switched-off account keeps its cache and shows greyed, but has no
//! workers, so it opens no connections; mail sent from it waits in the
//! Outbox until it is switched on again.

use super::text::{FormError, Proto};
use super::*;
use crate::Tr;
use crate::db::{CHECK_INTERVALS, Geometry, ThemeChoice};
use komorebi::i18n::{Lang, LangSetting, resolve_here};
use komorebi::theme::{self, SystemTheme};
use slint::Color;

/// The undo-send delays Settings offers, in seconds.
const UNDO_CHOICES: [u64; 5] = [0, 5, 10, 20, 30];
/// The mark-as-read delays Settings offers, as `mark_read_delay_ms`
/// values: at once, after 1 or 3 seconds, never.
const READ_CHOICES: [i64; 4] = [0, 1000, 3000, -1];
/// Every folder is synced in full at least this often, whatever the
/// check interval (the workers also force one every 60 passes).
pub(super) const FULL_SYNC_EVERY: Duration = Duration::from_secs(60 * 60);

/// The colours of the account dots, by account order (they repeat after
/// six). Mid-tone, so they read on light and dark rows alike.
const ACCOUNT_COLORS: [u32; 6] = [0x0f6cbd, 0xc239b3, 0x13a10e, 0xca5010, 0x8764b8, 0x038387];

/// The dot colour of the account at `index` in the account order.
pub(super) fn account_color(index: usize) -> Color {
    let rgb = ACCOUNT_COLORS[index % ACCOUNT_COLORS.len()];
    Color::from_argb_encoded(0xff00_0000 | rgb)
}

/// The offered value nearest to `value` (a setting saved by hand may lie
/// between them).
fn nearest(choices: &[u64], value: u64) -> usize {
    (0..choices.len())
        .min_by_key(|&i| choices[i].abs_diff(value))
        .unwrap_or(0)
}

/// The mark-as-read choice that shows for `mode`.
fn read_index(mode: MarkRead) -> usize {
    match mode {
        MarkRead::Now => 0,
        MarkRead::After(d) if d <= Duration::from_secs(2) => 1,
        MarkRead::After(_) => 2,
        MarkRead::Never => 3,
    }
}

fn theme_index(theme: ThemeChoice) -> i32 {
    match theme {
        ThemeChoice::System => 0,
        ThemeChoice::Light => 1,
        ThemeChoice::Dark => 2,
    }
}

/// WCAG 2 relative luminance of an opaque sRGB colour.
fn luminance(c: Color) -> f32 {
    theme::luminance([c.red(), c.green(), c.blue()])
}

/// The WCAG contrast ratio of two opaque colours (1 to 21).
fn contrast(a: Color, b: Color) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// `text` if it reads at 4.5:1 (WCAG AA) on `background`, else the first
/// mix of it towards black or white (whichever contrasts more with the
/// background) that does: the theme's accent-coloured text (`Theme.readable`).
pub(super) fn readable(text: Color, background: Color) -> Color {
    const AA: f32 = 4.5;
    let (black, white) = (
        Color::from_rgb_u8(0, 0, 0),
        Color::from_rgb_u8(255, 255, 255),
    );
    let target = if contrast(black, background) >= contrast(white, background) {
        black
    } else {
        white
    };
    let mix = |t: f32| {
        let channel =
            |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Color::from_rgb_u8(
            channel(text.red(), target.red()),
            channel(text.green(), target.green()),
            channel(text.blue(), target.blue()),
        )
    };
    (0..=50)
        .map(|step| mix(step as f32 / 50.0))
        .find(|&c| contrast(c, background) >= AA)
        .unwrap_or(target)
}

/// Show the Windows appearance settings `system` (only what differs from
/// `shown`, those last shown), under the theme choice `theme`.
pub(super) fn apply_system_theme(
    ui: &AppWindow,
    system: &SystemTheme,
    shown: Option<&SystemTheme>,
    theme: ThemeChoice,
) {
    set_tokens(ui, system, shown);
    apply_theme(ui, system, theme);
}

/// [`apply_system_theme`] for a message window: the same tokens and
/// light or dark, and its system caption dressed as the main window's
/// (it has no title bar of its own to switch).
pub(super) fn apply_popout_theme(ui: &AppWindow, system: &SystemTheme, theme: ThemeChoice) {
    set_tokens(ui, system, None);
    ui.global::<crate::Palette>()
        .set_color_scheme(color_scheme(system, theme));
    ui.set_custom_title_bar(false);
    crate::dress_popout(ui);
}

/// The Theme global's Windows inputs: the accent ramp, high contrast, the
/// animation switch and the text size.
fn set_tokens(ui: &AppWindow, system: &SystemTheme, shown: Option<&SystemTheme>) {
    let t = ui.global::<Theme>();
    let color = |c: theme::Rgb| Color::from_rgb_u8(c[0], c[1], c[2]);
    if shown.is_none_or(|s| s.ramp != system.ramp) {
        t.set_ramp(ModelRc::new(VecModel::from(
            system.ramp.map(color).to_vec(),
        )));
    }
    // Slint skips a set that changes nothing.
    t.set_contrast(system.contrast.is_some());
    if let Some(hc) = system.contrast {
        // Window, WindowText, Highlight, HighlightText, Hotlight, GrayText,
        // ButtonFace, ButtonText.
        let hc = hc.map(color);
        t.set_hc_window(hc[0]);
        t.set_hc_text(hc[1]);
        t.set_hc_highlight(hc[2]);
        t.set_hc_highlight_text(hc[3]);
        t.set_hc_hotlight(hc[4]);
        t.set_hc_gray(hc[5]);
        t.set_hc_button(hc[6]);
        t.set_hc_button_text(hc[7]);
    }
    t.set_animate(system.animate);
    t.set_caret_blink(i64::from(system.caret_blink_ms));
    let size = |px: f32| theme::scaled(px, system.text_scale);
    t.set_caption(size(12.0));
    t.set_body(size(14.0));
    t.set_subtitle(size(16.0));
    t.set_title(size(20.0));
    t.set_title_large(size(28.0));
    t.set_display(size(theme::DISPLAY));
}

/// Light or dark: under a contrast theme as its window colour is; else as
/// `theme` chooses, System following Windows.
fn color_scheme(system: &SystemTheme, theme: ThemeChoice) -> slint::language::ColorScheme {
    use slint::language::ColorScheme;
    let dark = match system.contrast {
        Some(hc) => theme::luminance(hc[0]) < 0.5,
        None => match theme {
            ThemeChoice::Light => false,
            ThemeChoice::Dark => true,
            ThemeChoice::System => system.dark,
        },
    };
    if dark {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    }
}

/// Show light or dark: under a contrast theme as its window colour is
/// (Slint's std widgets have no contrast colours, so they stay readable);
/// else as `theme` chooses, System following Windows. The title bar
/// follows (docs/WINUI3.md "Title bar and frame"): app-drawn on Windows 11,
/// the system caption under high contrast (switched live) or on Windows 10.
fn apply_theme(ui: &AppWindow, system: &SystemTheme, theme: ThemeChoice) {
    let scheme = color_scheme(system, theme);
    let dark = scheme == slint::language::ColorScheme::Dark;
    theme::set_menu_theme(dark && system.contrast.is_none());
    ui.global::<crate::Palette>().set_color_scheme(scheme);
    let layer = ui.global::<Theme>().get_layer();
    let caption = theme::caption_color(system, [layer.red(), layer.green(), layer.blue()]);
    let forced = komorebi::dev_var("KOMOREBI_SNAPSHOT_TITLEBAR").and_then(|v| v.into_string().ok());
    let custom = komorebi::chrome::custom_wanted(
        theme::windows_build(),
        system.contrast.is_some(),
        komorebi::dev_var("KOMOREBI_SYSTEM_TITLEBAR").is_some(),
        komorebi::dev_var("KOMOREBI_SNAPSHOT").is_some(),
        forced.as_deref(),
    );
    crate::dress_title_bar(
        ui,
        crate::TitleBar {
            dark: dark && system.contrast.is_none(),
            caption: caption.map(theme::to_colorref),
            custom,
        },
    );
}

impl App {
    // ------------------------------------------------------ workers

    /// Spawn the workers of every switched-on account that has none (they
    /// wait for their first job). Returns those accounts. A switched-off
    /// account gets none, so it opens no connections.
    pub(super) fn start_workers(&mut self) -> Vec<i64> {
        let new: Vec<Account> = self
            .accounts
            .iter()
            .filter(|a| a.enabled && !self.workers.contains_key(&a.id))
            .cloned()
            .collect();
        new.into_iter()
            .map(|a| {
                let id = a.id;
                self.spawn_workers(a);
                id
            })
            .collect()
    }

    /// Drop an account's workers: their channels close and the threads
    /// log out after the job at hand (and any queued before).
    pub(super) fn stop_workers(&mut self, id: i64) -> bool {
        let had = self.workers.remove(&id).is_some();
        self.syncing.remove(&id);
        self.warmed.remove(&id);
        self.account_state.remove(&id);
        self.ui().set_syncing(!self.syncing.is_empty());
        had
    }

    // ----------------------------------------------- account housekeeping

    /// The Enabled checkbox: switched off, the account's workers stop (a
    /// held move of its goes first); switched on, they start and sync.
    pub(super) fn set_account_enabled(&mut self, id: i64, on: bool) {
        if let Err(e) = self.db.set_account_enabled(id, on) {
            self.set_status(&text::detail(&e), true);
            return;
        }
        if !on {
            self.flush_held();
            self.stop_workers(id);
        }
        if let Err(e) = self.load_accounts() {
            self.set_status(&text::detail(&e), true);
        }
        if on && komorebi::dev_var("KOMOREBI_OFFLINE").is_none() {
            for id in self.start_workers() {
                self.workers[&id].send(Job::SyncAll { full: true });
            }
            self.drain_outbox();
        }
        self.reload_rows();
    }

    /// Up / Down beside an account: its place in the navigation and
    /// everywhere accounts are listed.
    pub(super) fn move_account(&mut self, id: i64, up: bool) {
        match self.db.move_account(id, up) {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => return self.set_status(&text::detail(&e), true),
        }
        if let Err(e) = self.load_accounts() {
            self.set_status(&text::detail(&e), true);
        }
        // The dots follow the order.
        self.reload_rows();
    }

    /// Re-download (after the prompt): the account's downloaded mail is
    /// forgotten and a full sync fetches it again. Its workers are
    /// replaced first, so no sync of the old state writes after the reset.
    /// The account, its password, drafts and Outbox, and the settings stay;
    /// nothing on the server changes.
    pub(super) fn redownload_now(&mut self, id: i64) {
        let email = self.account_email(id);
        self.flush_held();
        let had_workers = self.stop_workers(id);
        if self.open.as_ref().is_some_and(|o| o.msg.account_id == id) {
            self.close_reader();
        }
        if let Err(e) = self.db.redownload(id) {
            self.set_status(&t!(invoke_not_reset, format!("{e:#}").into()), true);
        }
        if let Err(e) = self.load_accounts() {
            self.set_status(&text::detail(&e), true);
        }
        self.reload_rows();
        self.refresh_message_tabs();
        if had_workers {
            self.respawn_workers(id);
            self.set_status(&t!(invoke_redownloading, email.as_str().into()), false);
        } else {
            self.set_status(&t!(invoke_redownload_later, email.as_str().into()), false);
        }
    }

    // ------------------------------------------------------- General

    /// Put the General choices into Settings: the theme, check interval,
    /// undo-send and mark-as-read choices (as indexes), and the flash.
    pub(super) fn show_general(&self) {
        let ui = self.ui();
        let choices = [
            theme_index(self.theme),
            nearest(&CHECK_INTERVALS, self.check_interval.as_secs()) as i32,
            nearest(&UNDO_CHOICES, self.undo_delay.as_secs()) as i32,
            read_index(self.mark_read) as i32,
        ];
        ui.set_g_choices(ModelRc::new(VecModel::from(choices.to_vec())));
        ui.set_g_flash(self.flash);
        ui.set_g_reopen_tabs(self.reopen_tabs);
        ui.set_g_language(self.language.index());
        ui.set_g_diag(komorebi::diag::enabled());
        self.show_notification_settings();
    }

    /// General choice `which` (0 theme, 1 check interval, 2 undo send,
    /// 3 mark as read, 4 taskbar flash, 5 the diagnostics log under
    /// Privacy, 6 the language: 0 System, 1 English, 2 Português
    /// (Portugal), 7 reopen tabs at start, 12 automatic update checks) changed to `value` (an index into its choices; for the
    /// switches 1 or 0): apply it and save it.
    pub(super) fn general_changed(&mut self, which: i32, value: i32) {
        let pick = |n: usize| (value.max(0) as usize).min(n - 1);
        let (key, saved) = match which {
            0 => {
                self.theme = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark][pick(3)];
                apply_theme(&self.ui(), &self.system, self.theme);
                self.theme_popouts();
                ("theme", self.theme.key().to_string())
            }
            1 => {
                let secs = CHECK_INTERVALS[pick(CHECK_INTERVALS.len())];
                self.check_interval = Duration::from_secs(secs);
                if self.started_sync {
                    self.start_sync_timer();
                }
                ("check_interval_secs", secs.to_string())
            }
            2 => {
                let secs = UNDO_CHOICES[pick(UNDO_CHOICES.len())];
                self.undo_delay = Duration::from_secs(secs);
                ("undo_send_secs", secs.to_string())
            }
            3 => {
                let ms = READ_CHOICES[pick(READ_CHOICES.len())];
                self.mark_read = match ms {
                    0 => MarkRead::Now,
                    ms if ms < 0 => MarkRead::Never,
                    ms => MarkRead::After(Duration::from_millis(ms as u64)),
                };
                ("mark_read_delay_ms", ms.to_string())
            }
            4 => {
                self.flash = value != 0;
                ("flash_taskbar", u8::from(self.flash).to_string())
            }
            6 => {
                self.language = LangSetting::from_index(value);
                self.language_changed();
                ("language", self.language.key().to_string())
            }
            5 => {
                let on = value != 0;
                komorebi::diag::set(on.then(|| komorebi::diag::log_path(&self.db_path)));
                ("diagnostics_log", u8::from(on).to_string())
            }
            // Off: the saved tabs go at once (ids only, but not kept).
            7 => {
                self.reopen_tabs = value != 0;
                self.save_tabs();
                ("reopen_tabs", u8::from(self.reopen_tabs).to_string())
            }
            8..=11 => {
                self.notification_changed(which, value != 0);
                return;
            }
            // Settings > General: Notifications for event reminders.
            13 => {
                self.set_reminders(value != 0);
                return;
            }
            // Settings > About: Check for updates automatically.
            12 => {
                self.set_update_auto(value != 0);
                return;
            }
            _ => return,
        };
        if let Err(e) = self.db.set_setting(key, &saved) {
            self.set_status(&t!(invoke_not_saved, format!("{e:#}").into()), true);
        }
    }

    /// The language choice changed: show the UI in it, now. Text in `.slint`
    /// follows by itself (`@tr` bindings); what Rust set (the window title,
    /// the folder list, the message rows) is set again here. A status line
    /// already on screen keeps its language until it is replaced. Every
    /// Rust-set text of the UI belongs in this method.
    pub(super) fn language_changed(&mut self) {
        let lang = resolve_here(self.language);
        self.lang = if komorebi::i18n::select(lang) {
            lang
        } else {
            Lang::En
        };
        self.rows.set_lang(self.lang);
        self.repush_text();
    }

    /// Set again every text Rust put in the UI, in the language now shown:
    /// what a binding in a `.slint` file doesn't follow by itself. Texts
    /// that keep a typed source (a problem, a draft, an outbox row, the
    /// open message) are made again from it; a status line already on
    /// screen says what it said until it is replaced, and so do the
    /// account form's one-off answers (`a_error`, `a_ok`, the lookup note).
    fn repush_text(&mut self) {
        self.push_window_title();
        self.repush_tray(self.inbox_unread.unwrap_or(0));
        self.show_last_updated();
        self.rebuild_nav();
        self.show_list_title();
        self.show_empty();
        // The rows read their text as they are drawn; a search of all
        // folders also carries the folders' names.
        if self.searching_all() {
            self.reload_rows();
        } else {
            self.rows.redraw();
        }
        self.show_account_rows();
        self.show_open();
        self.reader.language_changed();
        // Tab titles made by Rust ("New message", "(no subject)"), and
        // the message windows' Rust-set text.
        self.retitle_tabs();
        self.repush_popouts();
        self.show_outbox_entry();
        self.show_move_targets();
        self.repush_composer();
        let review = self.ui().get_a_review();
        self.show_cert_state(review);
        self.refresh_role_options();
        // The status line: a line that stands for an account out of reach
        // is made again; an offer (Undo, Retry...) keeps its button.
        let ui = self.ui();
        if ui.get_status_severity() == Severity::Caution && !self.standing.is_empty() {
            self.set_status("", false);
        }
        if self.status_action.get() != StatusAction::None {
            ui.set_status_action(action_label(self.status_action.get()).into());
        }
    }

    /// Windows' appearance settings changed (or may have): show them.
    pub(super) fn system_changed(&mut self, system: SystemTheme) {
        apply_system_theme(&self.ui(), &system, Some(&self.system), self.theme);
        self.system = system;
        // Every message window follows.
        self.theme_popouts();
    }

    // ----------------------------------------------- title and window

    /// The inboxes' unread total changed (or was counted again): the title
    /// shows it, and while the window is in the background a rise flashes
    /// the taskbar button if the user asked for that.
    pub(super) fn show_unread(&mut self, unread: i32) {
        let before = self.inbox_unread.replace(unread);
        // New mail is found by message id, not by the unread total: a
        // message read in the same refresh must not hide a new one.
        if before.is_some() {
            self.notify_new_mail();
        }
        if before == Some(unread) {
            return;
        }
        let ui = self.ui();
        self.push_window_title();
        if self.flash && !self.focused && before.is_some_and(|b| unread > b) {
            crate::flash_taskbar(&ui);
        }
        self.repush_tray(unread);
    }

    /// Put the title in the window: the inboxes' unread count, if any, in
    /// the language shown.
    pub(super) fn push_window_title(&self) {
        let ui = self.ui();
        let unread = self.inbox_unread.unwrap_or(0);
        ui.set_window_title(ui.global::<Tr>().invoke_window_title(unread));
    }

    /// Save where the window is, for the next start (as it closes).
    pub(super) fn save_geometry(&self) {
        let Some(normal) = self.normal_geometry else {
            return;
        };
        let g = Geometry {
            maximized: self.ui().window().is_maximized(),
            ..normal
        };
        self.db.set_setting("window", &g.to_setting()).ok();
    }
}

/// What a server name field may hold, checked and normalised: the name
/// (or IP address) only, no scheme, port, path or user. Trimmed, one
/// trailing dot dropped, lower case, an IPv6 literal unbracketed (see
/// [`host_for_display`]) and a Unicode name in its `xn--` form. The error
/// says what is wrong ([`text::form_error`] words it).
pub(super) fn validate_host(raw: &str) -> Result<String, FormError> {
    let name_only = || FormError::ServerNameOnly;
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(FormError::ServerNames);
    }
    if raw.chars().any(|c| c.is_whitespace() || c.is_control())
        || raw.contains(['/', '\\', '@', '?', '#'])
    {
        return Err(name_only());
    }
    // An IPv6 literal, with or without brackets.
    let inner = match raw.strip_prefix('[') {
        Some(rest) => rest.strip_suffix(']').ok_or_else(name_only)?,
        None => raw,
    };
    if inner.contains(':') {
        if inner.contains('%') {
            return Err(FormError::ZoneId);
        }
        return inner
            .parse::<std::net::Ipv6Addr>()
            .map(|ip| ip.to_string())
            .map_err(|_| name_only());
    }
    if raw.starts_with('[') {
        return Err(name_only());
    }
    // A name: one trailing dot is the root, not part of the name.
    let name = raw.strip_suffix('.').unwrap_or(raw);
    if name.is_empty() || name.ends_with('.') {
        return Err(name_only());
    }
    if name.parse::<std::net::Ipv4Addr>().is_ok() {
        return Ok(name.to_string());
    }
    let ascii = if name.is_ascii() {
        name.to_ascii_lowercase()
    } else {
        idna::domain_to_ascii(name).map_err(|_| FormError::AsciiName)?
    };
    let label_ok = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    };
    if ascii.len() > 253 || !ascii.split('.').all(label_ok) {
        return Err(name_only());
    }
    Ok(ascii)
}

/// A port as typed in the form: 1 to 65535. The one place that decides, for
/// the checks (which say so with [`FormError::Port`]) and for the certificate
/// lines (which wait until the port is right).
pub(super) fn parse_port(raw: &str) -> Option<u16> {
    raw.trim().parse::<u16>().ok().filter(|p| *p != 0)
}

/// A stored server name as the form shows it: an IPv6 address in brackets.
pub(super) fn host_for_display(host: &str) -> String {
    if host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

/// The default port of a server (IMAP when `imap`, else SMTP) for `security`.
fn default_port(imap: bool, security: Security) -> &'static str {
    match (imap, security) {
        (true, Security::Tls) => "993",
        (true, Security::StartTls) => "143",
        (false, Security::Tls) => "465",
        (false, Security::StartTls) => "587",
    }
}

/// The port field after the security changed from `old` to `new`: the new
/// default if the field was empty or still held the old default, else what
/// the user typed (a custom port such as 1143 or 2525 survives).
pub(super) fn port_for_security(imap: bool, old: Security, new: Security, port: &str) -> String {
    let port = port.trim();
    if port.is_empty() || port == default_port(imap, old) {
        default_port(imap, new).into()
    } else {
        port.into()
    }
}

/// The sign-in names a form holds, as stored: a name equal to the address
/// is empty, and the SMTP one follows the IMAP one unless the form has its
/// own. A preset keeps the username typed under the password ("Set
/// username"), for both servers: its separate SMTP field is not shown.
pub(super) fn form_logins(form: &AccountForm, email: &str) -> Result<(String, String), FormError> {
    let preset = usize::try_from(form.provider)
        .ok()
        .and_then(|i| providers::PRESETS.get(i))
        .is_some();
    let clean = |raw: &str, what: Proto| -> Result<String, FormError> {
        let user = raw.trim();
        if user.chars().any(char::is_control) || user.len() > 254 {
            return Err(FormError::Username(what));
        }
        Ok(if user == email {
            String::new()
        } else {
            user.into()
        })
    };
    let imap = clean(&form.imap_user, Proto::Imap)?;
    let smtp = if form.smtp_own_login && !preset {
        clean(&form.smtp_user, Proto::Smtp)?
    } else {
        imap.clone()
    };
    Ok((imap, smtp))
}

/// Whether the form's SMTP server takes mail without a sign-in: only a
/// manual server can (a preset always signs in).
pub(super) fn form_no_auth(form: &AccountForm) -> bool {
    let preset = usize::try_from(form.provider)
        .ok()
        .and_then(|i| providers::PRESETS.get(i))
        .is_some();
    form.smtp_no_auth && !preset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_show_the_nearest_offered_value() {
        assert_eq!(nearest(&UNDO_CHOICES, 5), 1);
        assert_eq!(nearest(&UNDO_CHOICES, 7), 1);
        assert_eq!(nearest(&UNDO_CHOICES, 60), 4);
        assert_eq!(nearest(&CHECK_INTERVALS, 300), 2);
        assert_eq!(read_index(MarkRead::Now), 0);
        assert_eq!(read_index(MarkRead::After(Duration::from_secs(1))), 1);
        assert_eq!(read_index(MarkRead::After(Duration::from_secs(60))), 2);
        assert_eq!(read_index(MarkRead::Never), 3);
        // Every offered delay shows as itself.
        for (i, ms) in READ_CHOICES.iter().enumerate() {
            let mode = match *ms {
                0 => MarkRead::Now,
                ms if ms < 0 => MarkRead::Never,
                ms => MarkRead::After(Duration::from_millis(ms as u64)),
            };
            assert_eq!(read_index(mode), i);
        }
    }

    /// A colour of `ui/theme.slint`: `name: dark ? #dark : #light;`.
    fn theme_color(name: &str, dark: bool) -> Color {
        let theme = include_str!("../../ui/theme.slint");
        // `name: contrast ? … : dark ? #dark : #light;`
        let line = theme
            .lines()
            .find(|l| l.contains(&format!(" {name}: ")) && l.contains(" dark ? #"))
            .unwrap_or_else(|| panic!("{name} in theme.slint"));
        let (_, values) = line.split_once(" dark ? ").unwrap();
        let hex: Vec<&str> = values.split('#').skip(1).map(|h| &h[..6]).collect();
        let rgb = u32::from_str_radix(hex[if dark { 0 } else { 1 }], 16).unwrap();
        Color::from_argb_encoded(0xff00_0000 | rgb)
    }

    #[test]
    fn accent_text_reads_on_every_surface() {
        // Windows accent colours, the light and the hard ones.
        let accents = [
            0x0078d4, 0x005fb8, 0x60cdff, 0xffb900, 0xfff100, 0xbad80a, 0x00b7c3, 0x00cc6a,
            0xe3008c, 0xff8c00, 0xe81123, 0x767676, 0x881798, 0x101010, 0xf0f0f0,
        ];
        for dark in [false, true] {
            let surfaces = ["card", "nav-selected", "layer"].map(|n| theme_color(n, dark));
            for rgb in accents {
                let accent = Color::from_argb_encoded(0xff00_0000 | rgb);
                // As Theme.accent-text derives it from the ramp's Dark2
                // (light) or Light3 (dark).
                let text = readable(readable(accent, surfaces[0]), surfaces[1]);
                for bg in surfaces {
                    let ratio = contrast(text, bg);
                    assert!(
                        ratio >= 4.5,
                        "{rgb:06x} on {bg:?} (dark {dark}): {ratio:.2}"
                    );
                }
            }
        }
        // A colour that already reads is kept as it is.
        let blue = Color::from_rgb_u8(0x00, 0x5f, 0xb8);
        assert_eq!(readable(blue, theme_color("card", false)), blue);
        assert!(
            (contrast(
                Color::from_rgb_u8(0, 0, 0),
                Color::from_rgb_u8(255, 255, 255)
            ) - 21.0)
                .abs()
                < 0.01
        );
    }

    #[test]
    fn account_dots_repeat_after_six() {
        assert_eq!(account_color(0), account_color(6));
        assert_ne!(account_color(0), account_color(1));
        assert_eq!(account_color(0).alpha(), 255);
    }

    #[test]
    fn server_names_are_checked_and_normalised() {
        let ok =
            |raw: &str, want: &str| assert_eq!(validate_host(raw).as_deref(), Ok(want), "{raw}");
        ok("mail.example.com", "mail.example.com");
        ok("  Mail.Example.COM  ", "mail.example.com");
        ok("mail.example.com.", "mail.example.com");
        ok("localhost", "localhost");
        ok("intranet", "intranet");
        ok("mail_1.example.com", "mail_1.example.com");
        ok("192.168.1.10", "192.168.1.10");
        ok("127.0.0.1", "127.0.0.1");
        ok("::1", "::1");
        ok("[::1]", "::1");
        ok("[2001:DB8::1]", "2001:db8::1");
        ok("2001:db8::1", "2001:db8::1");
        ok("bücher.example", "xn--bcher-kva.example");

        let name_only = FormError::ServerNameOnly;
        for bad in [
            "imap://mail.example.com",
            "https://mail.example.com/",
            "mail.example.com/path",
            "mail.example.com\\x",
            "user@mail.example.com",
            "mail.example.com:993",
            "mail example.com",
            "mail\n.example.com",
            "mail..example.com",
            ".mail.example.com",
            "mail.example.com..",
            "-",
            "[::1",
            "::1]",
            "[mail.example.com]",
            "[1:2:3]",
            "mail!.example.com",
            "mail.example.com?x=1",
            "a b",
            ".",
        ] {
            assert_eq!(validate_host(bad), Err(name_only), "{bad:?}");
        }
        assert_eq!(validate_host(""), Err(FormError::ServerNames));
        assert!(validate_host("   ").is_err());
        assert_eq!(validate_host("fe80::1%eth0"), Err(FormError::ZoneId));
        assert_eq!(validate_host("[fe80::1%25eth0]"), Err(FormError::ZoneId));
        assert!(validate_host(&format!("{}.com", "a".repeat(64))).is_err());
        assert!(validate_host(&"a.".repeat(130)).is_err());
        // Bracketed for display only.
        assert_eq!(host_for_display("::1"), "[::1]");
        assert_eq!(host_for_display("mail.example.com"), "mail.example.com");
        assert_eq!(host_for_display("10.0.0.1"), "10.0.0.1");
    }

    #[test]
    fn a_security_change_moves_only_a_default_port() {
        use Security::{StartTls, Tls};
        let p = port_for_security;
        assert_eq!(p(true, Tls, StartTls, "993"), "143");
        assert_eq!(p(true, StartTls, Tls, "143"), "993");
        assert_eq!(p(false, Tls, StartTls, "465"), "587");
        assert_eq!(p(false, StartTls, Tls, "587"), "465");
        // A custom port stays: Proton Bridge, submission on 2525 or 25.
        assert_eq!(p(true, Tls, StartTls, "1143"), "1143");
        assert_eq!(p(false, StartTls, Tls, "2525"), "2525");
        assert_eq!(p(false, StartTls, Tls, "25"), "25");
        // The other server's default is custom here.
        assert_eq!(p(true, Tls, StartTls, "465"), "465");
        assert_eq!(p(false, Tls, StartTls, "993"), "993");
        // An empty field takes the new default; whitespace is ignored.
        assert_eq!(p(true, Tls, StartTls, ""), "143");
        assert_eq!(p(true, Tls, StartTls, " 993 "), "143");
        // Same security: nothing to move.
        assert_eq!(p(true, Tls, Tls, "993"), "993");
    }

    #[test]
    fn form_logins_follow_the_form() {
        let manual = AccountForm {
            provider: providers::PRESETS.len() as i32,
            imap_user: " alice ".into(),
            ..Default::default()
        };
        let logins = |f: &AccountForm| form_logins(f, "alice@example.com");
        // The SMTP login follows the IMAP one unless the form has its own.
        assert_eq!(logins(&manual), Ok(("alice".into(), "alice".into())));
        let own = AccountForm {
            smtp_own_login: true,
            smtp_user: "relay".into(),
            ..manual.clone()
        };
        assert_eq!(logins(&own), Ok(("alice".into(), "relay".into())));
        // An own SMTP login left empty is the address.
        let blank = AccountForm {
            smtp_own_login: true,
            smtp_user: " ".into(),
            ..manual.clone()
        };
        assert_eq!(logins(&blank), Ok(("alice".into(), String::new())));
        // Empty, or the address itself, stores nothing.
        let none = AccountForm {
            imap_user: "alice@example.com".into(),
            ..manual.clone()
        };
        assert_eq!(logins(&none), Ok((String::new(), String::new())));
        // A preset keeps a username typed under the password (Set
        // username), for both servers; the SMTP field it doesn't show is
        // ignored.
        let preset = AccountForm {
            provider: 0,
            ..own.clone()
        };
        assert_eq!(logins(&preset), Ok(("alice".into(), "alice".into())));
        let preset_plain = AccountForm {
            provider: 1,
            imap_user: "".into(),
            ..own.clone()
        };
        assert_eq!(logins(&preset_plain), Ok((String::new(), String::new())));
        let bad = AccountForm {
            imap_user: "a\nb".into(),
            ..manual
        };
        assert_eq!(logins(&bad), Err(FormError::Username(Proto::Imap)));
    }
}
