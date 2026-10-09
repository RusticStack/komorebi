//! Settings beyond the account form: the General choices (theme, check
//! interval, undo-send and mark-as-read delays, taskbar flash), the
//! diagnostics log switch (Privacy), account
//! housekeeping (Enabled, order, Re-download), the window title's unread
//! count and the window's place between runs.
//!
//! A switched-off account keeps its cache and shows greyed, but has no
//! workers, so it opens no connections; mail sent from it waits in the
//! Outbox until it is switched on again.

use super::*;
use crate::db::{CHECK_INTERVALS, Geometry, ThemeChoice};
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
    let linear = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(c.red()) + 0.7152 * linear(c.green()) + 0.0722 * linear(c.blue())
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

/// Show `theme` (System follows Windows: `system_dark`, when known; before
/// the window exists nothing is set, so the style keeps tracking it).
pub(super) fn apply_theme(ui: &AppWindow, theme: ThemeChoice, system_dark: Option<bool>) {
    use slint::language::ColorScheme;
    let dark = match theme {
        ThemeChoice::Light => false,
        ThemeChoice::Dark => true,
        ThemeChoice::System => match system_dark {
            Some(dark) => dark,
            None => return,
        },
    };
    let scheme = if dark {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    };
    ui.global::<crate::Palette>().set_color_scheme(scheme);
    crate::dark_title_bar(ui, dark);
}

/// Whether Windows uses its dark app theme, from the window (none before
/// it exists, or without winit: tests).
fn system_dark(ui: &AppWindow) -> Option<bool> {
    use slint::winit_030::{WinitWindowAccessor, winit::window::Theme};
    ui.window()
        .with_winit_window(|w| w.theme())
        .flatten()
        .map(|t| t == Theme::Dark)
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
            self.set_status(&format!("{e:#}"), true);
            return;
        }
        if !on {
            self.flush_held();
            self.stop_workers(id);
        }
        if let Err(e) = self.load_accounts() {
            self.set_status(&format!("{e:#}"), true);
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
            Err(e) => return self.set_status(&format!("{e:#}"), true),
        }
        if let Err(e) = self.load_accounts() {
            self.set_status(&format!("{e:#}"), true);
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
            self.set_status(&format!("Not reset. {e:#}"), true);
        }
        if let Err(e) = self.load_accounts() {
            self.set_status(&format!("{e:#}"), true);
        }
        self.reload_rows();
        if had_workers {
            self.respawn_workers(id);
            self.set_status(&format!("Downloading the mail of {email} again…"), false);
        } else {
            self.set_status(
                &format!("The mail of {email} downloads again at its next sync."),
                false,
            );
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
        ui.set_g_diag(komorebi::diag::enabled());
    }

    /// General choice `which` (0 theme, 1 check interval, 2 undo send,
    /// 3 mark as read, 4 taskbar flash, 5 the diagnostics log under
    /// Privacy) changed to `value` (an index into its choices; for the
    /// switches 1 or 0): apply it and save it.
    pub(super) fn general_changed(&mut self, which: i32, value: i32) {
        let pick = |n: usize| (value.max(0) as usize).min(n - 1);
        let (key, saved) = match which {
            0 => {
                self.theme = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark][pick(3)];
                let ui = self.ui();
                apply_theme(&ui, self.theme, system_dark(&ui));
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
            5 => {
                let on = value != 0;
                komorebi::diag::set(on.then(|| komorebi::diag::log_path(&self.db_path)));
                ("diagnostics_log", u8::from(on).to_string())
            }
            _ => return,
        };
        if let Err(e) = self.db.set_setting(key, &saved) {
            self.set_status(&format!("Not saved. {e:#}"), true);
        }
    }

    /// Windows switched between light and dark: the System choice follows.
    pub(super) fn system_theme(&self, dark: bool) {
        if self.theme == ThemeChoice::System {
            apply_theme(&self.ui(), ThemeChoice::System, Some(dark));
        } else {
            // winit has just set the title bar to the Windows theme.
            self.refresh_title_bar();
        }
    }

    /// Match the title bar to the theme shown again: once the window
    /// exists, and when it comes to the front (winit resets the title bar
    /// to the Windows theme whenever a system setting changes).
    pub(super) fn refresh_title_bar(&self) {
        use slint::language::ColorScheme;
        let ui = self.ui();
        let scheme = ui.global::<crate::Palette>().get_color_scheme();
        crate::dark_title_bar(&ui, scheme == ColorScheme::Dark);
    }

    // ----------------------------------------------- title and window

    /// The inboxes' unread total changed (or was counted again): the title
    /// shows it, and while the window is in the background a rise flashes
    /// the taskbar button if the user asked for that.
    pub(super) fn show_unread(&mut self, unread: i32) {
        let before = self.inbox_unread.replace(unread);
        if before == Some(unread) {
            return;
        }
        let ui = self.ui();
        ui.set_window_title(crate::window_title(unread).into());
        if self.flash && !self.focused && before.is_some_and(|b| unread > b) {
            crate::flash_taskbar(&ui);
        }
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
        let line = theme
            .lines()
            .find(|l| l.contains(&format!(" {name}: dark ?")))
            .unwrap_or_else(|| panic!("{name} in theme.slint"));
        let hex: Vec<&str> = line.split('#').skip(1).map(|h| &h[..6]).collect();
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
            let surfaces =
                ["row-selected", "nav-selected", "card", "layer"].map(|n| theme_color(n, dark));
            for rgb in accents {
                let accent = Color::from_argb_encoded(0xff00_0000 | rgb);
                // As Theme.accent-text derives it.
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
}
