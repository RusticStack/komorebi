//! Settings > General (the Slint app's `general_changed`, `show_general`
//! and `language_changed`): the theme, the language, how often mail is
//! looked for, undo send, mark as read, the taskbar flash, notifications,
//! the tray and start with Windows, reopen tabs and how new messages start
//! (the format, and the default font, size and colour of formatted ones,
//! the Slint app's `compose_defaults_changed`).
//! Theme and language apply at once; the rest is saved and told to the
//! shell ([`Setting`]).

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement, Role, SharedString, Styled, div,
    prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use komorebi::db::ThemeChoice;
use komorebi::i18n::{LangSetting, resolve_here};
use komorebi::mail::rich::{ColorId, FontId, SizePt};
use komorebi_i18n::tr;

use super::card::{CARD_MAX_W, Card, Segmented, group_head, page_head};
use super::choices::{INTERVALS, READ_CHOICES, THEMES, UNDO_CHOICES, nearest, read_index};
use super::{Setting, SettingsEvent, SettingsView};
use crate::icons::Icon;
use crate::theme::settings as geo;
use crate::theme::{ActiveTokens as _, TextStyled as _, radius, space};
use crate::views::composer::text::words;
use crate::widgets::select::Dropdown;
use crate::widgets::toggle::Toggle;

/// Whether Windows starts Komorebi at sign-in.
pub fn autostart_enabled() -> bool {
    #[cfg(windows)]
    return komorebi::win_shell::autostart_enabled();
    #[cfg(not(windows))]
    false
}

fn set_autostart(on: bool) {
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        komorebi::win_shell::set_autostart(on, &exe);
    }
    #[cfg(not(windows))]
    let _ = on;
}

fn theme_label(theme: ThemeChoice) -> String {
    match theme {
        ThemeChoice::System => tr!("settings theme" => "System"),
        ThemeChoice::Light => tr!("Light"),
        ThemeChoice::Dark => tr!("Dark"),
    }
}

fn interval_label(secs: u64) -> String {
    match secs {
        60 => tr!("Every minute"),
        120 => tr!("Every 2 minutes"),
        300 => tr!("Every 5 minutes"),
        _ => tr!("Every 15 minutes"),
    }
}

fn undo_label(secs: u64) -> String {
    match secs {
        0 => tr!("Off"),
        5 => tr!("5 seconds"),
        10 => tr!("10 seconds"),
        20 => tr!("20 seconds"),
        _ => tr!("30 seconds"),
    }
}

fn read_label(ms: i64) -> String {
    match ms {
        0 => tr!("Right away"),
        1000 => tr!("After 1 second"),
        3000 => tr!("After 3 seconds"),
        _ => tr!("Never"),
    }
}

pub(crate) fn language_label(lang: LangSetting) -> String {
    match lang {
        LangSetting::System => tr!("settings language" => "System"),
        LangSetting::English => "English".to_string(),
        LangSetting::PortuguesePortugal => "Português (Portugal)".to_string(),
    }
}

/// The default font's three choices as saved (`compose_font`: the font's
/// label, `compose_size`: points, `compose_color`: `#RRGGBB` or `auto`).
pub fn font_values(
    font: FontId,
    size: SizePt,
    color: Option<ColorId>,
) -> [(&'static str, String); 3] {
    [
        ("compose_font", font.label().to_string()),
        ("compose_size", size.get().to_string()),
        (
            "compose_color",
            color.map_or_else(|| "auto".into(), |c| c.hex().to_string()),
        ),
    ]
}

pub(crate) const LANGUAGES: [LangSetting; 3] = [
    LangSetting::System,
    LangSetting::English,
    LangSetting::PortuguesePortugal,
];

impl SettingsView {
    /// The theme choice changed: applied to every window, then saved.
    pub fn set_theme(&mut self, theme: ThemeChoice, cx: &mut Context<Self>) {
        crate::theme::set_choice(theme, cx);
        self.save("theme", theme.key(), cx);
    }

    /// The language choice changed: shown now (every window follows), then
    /// saved. `System` follows the Windows display language.
    pub fn set_language(&mut self, setting: LangSetting, cx: &mut Context<Self>) {
        let lang = resolve_here(setting);
        crate::i18n::set_language(lang, cx);
        self.save("language", setting.key(), cx);
    }

    /// A General choice by its index among the choices offered.
    pub fn set_setting(&mut self, setting: Setting, cx: &mut Context<Self>) {
        let (key, value) = match &setting {
            Setting::CheckInterval(secs) => ("check_interval_secs", secs.to_string()),
            Setting::UndoSend(secs) => ("undo_send_secs", secs.to_string()),
            Setting::MarkRead(ms) => ("mark_read_delay_ms", ms.to_string()),
            Setting::FlashTaskbar(on) => ("flash_taskbar", u8::from(*on).to_string()),
            Setting::Notify(on) => ("notify", u8::from(*on).to_string()),
            Setting::NotifyDetail(on) => ("notify_detail", u8::from(*on).to_string()),
            Setting::CloseToTray(on) => ("close_to_tray", u8::from(*on).to_string()),
            Setting::ReopenTabs(on) => ("reopen_tabs", u8::from(*on).to_string()),
            Setting::DiagnosticsLog(on) => ("diagnostics_log", u8::from(*on).to_string()),
            Setting::UpdateAuto(on) => ("update_auto", u8::from(*on).to_string()),
            Setting::ComposeRich(on) => {
                ("compose_format", if *on { "rich" } else { "plain" }.into())
            }
            Setting::Autostart(on) => {
                // The Run key, not the cache.
                set_autostart(*on);
                cx.emit(SettingsEvent::Changed(setting));
                cx.notify();
                return;
            }
            Setting::AccountNotify { .. } => return,
        };
        if self.save(key, &value, cx) {
            cx.emit(SettingsEvent::Changed(setting));
        }
    }

    /// General > Compose > Default font: the font, size and colour formatted
    /// messages start in. Saved, then told to the shell as the compose
    /// settings (the composer reads them again).
    pub fn set_compose_font(
        &mut self,
        font: FontId,
        size: SizePt,
        color: Option<ColorId>,
        cx: &mut Context<Self>,
    ) {
        for (key, value) in font_values(font, size, color) {
            if !self.save(key, &value, cx) {
                return;
            }
        }
        cx.emit(SettingsEvent::Changed(Setting::ComposeRich(
            self.s.compose_rich,
        )));
    }

    /// The Default font card: three boxes and a sample line in the font,
    /// size and colour chosen (the colour in every theme, as in the Slint
    /// card; no colour is the ink).
    fn font_card(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let c = t.color;
        let (font, size, color) = (
            self.s.compose_font,
            self.s.compose_size,
            self.s.compose_color,
        );
        let font_box = Dropdown::new("settings-font", words::font(), font.label())
            .choices(FontId::ALL.iter().map(|f| (f.label(), f.label())))
            .on_inset(true)
            .on_pick(cx.listener(move |this, value: &SharedString, _, cx| {
                if let Some(f) = FontId::ALL.iter().find(|f| f.label() == value.as_ref()) {
                    this.set_compose_font(*f, size, color, cx);
                }
            }));
        let size_box = Dropdown::new(
            "settings-font-size",
            words::font_size(),
            size.get().to_string(),
        )
        .choices(
            SizePt::ALL
                .iter()
                .map(|pt| (pt.to_string(), pt.to_string())),
        )
        .on_inset(true)
        .on_pick(cx.listener(move |this, value: &SharedString, _, cx| {
            if let Some(pt) = value.parse::<u8>().ok().and_then(SizePt::new) {
                this.set_compose_font(font, pt, color, cx);
            }
        }));
        let colour_key = |c: Option<ColorId>| c.map_or("auto", ColorId::hex);
        let colour_box = Dropdown::new(
            "settings-font-colour",
            words::font_colour(),
            colour_key(color),
        )
        .choices(
            std::iter::once(None)
                .chain(ColorId::ALL.iter().copied().map(Some))
                .map(|c| (colour_key(c), words::colour(c))),
        )
        .on_inset(true)
        .on_pick(cx.listener(move |this, value: &SharedString, _, cx| {
            let picked = ColorId::ALL
                .iter()
                .copied()
                .find(|c| c.hex() == value.as_ref());
            this.set_compose_font(font, size, picked, cx);
        }));
        let sample = match color {
            // The swatch in every theme, as the Slint card shows it (its
            // only exception is high contrast, not in 1.0).
            Some(colour) => {
                let (r, g, b) = colour.rgb(); // token-exempt: the saved compose colour, not a literal
                crate::theme::reader::page_colour([r, g, b, u8::MAX])
            }
            _ => c.ink,
        };
        div()
            .id("settings-default-font")
            .role(Role::Group)
            .aria_label(tr!("Default font"))
            .w_full()
            .max_w(px(CARD_MAX_W))
            .mb(px(space::SP_1))
            .p(px(space::SP_4))
            .flex()
            .flex_row()
            .items_start()
            .gap(px(space::SP_4))
            .rounded(px(radius::CARD))
            .bg(c.inset)
            // The card's icon, as every card of the page has one, so the
            // titles make one left edge (`card::Card`).
            .child(super::card::card_icon(Icon::TextSize, &t))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(space::SP_2))
                    .child(div().typo(&t.text.body).child(tr!("Default font")))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(space::SP_2))
                            .child(div().w(px(geo::FONT_BOX_W)).child(font_box))
                            .child(div().w(px(geo::SIZE_BOX_W)).child(size_box))
                            .child(div().w(px(geo::FONT_BOX_W)).child(colour_box)),
                    )
                    .child(
                        div()
                            .font_family(font.label())
                            .text_size(px(f32::from(size.get()) * geo::PT_PX))
                            .text_color(sample)
                            .child(tr!("The quick brown fox jumps over the lazy dog.")),
                    ),
            )
            .test_support()
    }

    #[allow(clippy::too_many_arguments)] // a card's words, its page and its view
    fn choice_card(
        &self,
        id: &'static str,
        title: String,
        note: String,
        current: String,
        choices: Vec<(String, String)>,
        cx: &mut Context<Self>,
        pick: impl Fn(&mut Self, &str, &mut Context<Self>) + 'static,
    ) -> Card {
        Card::new(id, title.clone()).note(note).control(
            Dropdown::new(id, title, current)
                .choices(choices)
                .on_inset(true)
                .on_pick(
                    cx.listener(move |this, value: &SharedString, _, cx| pick(this, value, cx)),
                ),
        )
    }

    fn toggle_card(
        &self,
        id: &'static str,
        title: String,
        note: String,
        on: bool,
        cx: &mut Context<Self>,
        change: impl Fn(&mut Self, bool, &mut Context<Self>) + 'static,
    ) -> Card {
        let this = cx.entity().downgrade();
        Card::new(id, title.clone()).note(note).control(
            Toggle::new(id, title, on)
                .state_label(tr!("On"), tr!("Off"))
                .on_change(move |on, _, cx| {
                    let _ = this.update(cx, |this, cx| change(this, on, cx));
                }),
        )
    }

    pub(crate) fn render_general(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let s = self.s.clone();
        // The mockup's segmented control (`.seg`): three short choices.
        let theme = Card::new("settings-theme", tr!("Theme"))
            .note(tr!("System follows your Windows app mode."))
            .control(
                Segmented::new("settings-theme-choice", tr!("Theme"), s.theme.key())
                    .choices(
                        THEMES
                            .iter()
                            .map(|t| (t.key().to_string(), theme_label(*t))),
                    )
                    .on_pick(cx.listener(|this, value: &SharedString, _, cx| {
                        if let Some(t) = THEMES.iter().find(|t| t.key() == value.as_ref()) {
                            this.set_theme(*t, cx);
                        }
                    })),
            );
        let language = self.choice_card(
            "settings-language",
            tr!("settings language" => "Language"),
            tr!("settings language" => "The language of menus and messages. System follows your Windows display language."),
            s.language.key().to_string(),
            LANGUAGES
                .iter()
                .map(|l| (l.key().to_string(), language_label(*l)))
                .collect(),
            cx,
            |this, value, cx| {
                if let Some(l) = LANGUAGES.iter().find(|l| l.key() == value) {
                    this.set_language(*l, cx);
                }
            },
        );
        let interval = INTERVALS[nearest(&INTERVALS, s.check_interval_secs)];
        let check = self.choice_card(
            "settings-check",
            tr!("Check for new mail"),
            tr!("Servers that push new mail show it within seconds whatever the interval."),
            interval.to_string(),
            INTERVALS
                .iter()
                .map(|s| (s.to_string(), interval_label(*s)))
                .collect(),
            cx,
            |this, value, cx| {
                if let Ok(secs) = value.parse() {
                    this.set_setting(Setting::CheckInterval(secs), cx);
                }
            },
        );
        let undo_now = UNDO_CHOICES[nearest(&UNDO_CHOICES, s.undo_send_secs)];
        let undo = self.choice_card(
            "settings-undo",
            tr!("Undo send"),
            tr!("Holds a sent message this long so it can be taken back."),
            undo_now.to_string(),
            UNDO_CHOICES
                .iter()
                .map(|s| (s.to_string(), undo_label(*s)))
                .collect(),
            cx,
            |this, value, cx| {
                if let Ok(secs) = value.parse() {
                    this.set_setting(Setting::UndoSend(secs), cx);
                }
            },
        );
        let read_now = READ_CHOICES[read_index(s.mark_read)];
        let read = self.choice_card(
            "settings-read",
            tr!("Mark as read"),
            tr!("When a message opens in the reader."),
            read_now.to_string(),
            READ_CHOICES
                .iter()
                .map(|ms| (ms.to_string(), read_label(*ms)))
                .collect(),
            cx,
            |this, value, cx| {
                if let Ok(ms) = value.parse() {
                    this.set_setting(Setting::MarkRead(ms), cx);
                }
            },
        );
        let flash = self
            .toggle_card(
                "settings-flash",
                tr!("Flash the taskbar button"),
                tr!("When new mail arrives while Komorebi is in the background."),
                s.flash_taskbar,
                cx,
                |this, on, cx| this.set_setting(Setting::FlashTaskbar(on), cx),
            )
            .icon(Icon::Flash);
        let notify = self.toggle_card(
            "settings-notify",
            tr!("Notifications for new mail"),
            tr!("Windows shows a notification when new mail arrives while Komorebi is in the background. They are made on this computer; nothing is sent anywhere."),
            s.notify,
            cx,
            |this, on, cx| this.set_setting(Setting::Notify(on), cx),
        );
        let detail = self.toggle_card(
            "settings-notify-detail",
            tr!("Show the sender and subject"),
            tr!("Off: a notification only says New mail, which is safer on a shared screen or the lock screen."),
            s.notify_detail,
            cx,
            |this, on, cx| this.set_setting(Setting::NotifyDetail(on), cx),
        );
        let tray = self.toggle_card(
            "settings-tray",
            tr!("Keep running in the notification area"),
            tr!("Closing the window keeps Komorebi in the notification area, so mail keeps arriving. Quit from its icon's menu."),
            s.close_to_tray,
            cx,
            |this, on, cx| this.set_setting(Setting::CloseToTray(on), cx),
        );
        let autostart = self.toggle_card(
            "settings-autostart",
            tr!("Start with Windows"),
            tr!("Komorebi starts in the notification area when you sign in to Windows."),
            autostart_enabled(),
            cx,
            |this, on, cx| this.set_setting(Setting::Autostart(on), cx),
        );
        let tabs = self.toggle_card(
            "settings-tabs",
            tr!("Reopen tabs at start"),
            tr!("The message and draft tabs open when Komorebi closes come back the next time."),
            s.reopen_tabs,
            cx,
            |this, on, cx| this.set_setting(Setting::ReopenTabs(on), cx),
        );
        let format = self.choice_card(
            "settings-format",
            tr!("Message format"),
            tr!("How new messages start. Any message can be switched in its More menu."),
            if s.compose_rich { "rich" } else { "plain" }.to_string(),
            vec![
                ("rich".to_string(), tr!("Formatted (HTML)")),
                ("plain".to_string(), tr!("Plain text")),
            ],
            cx,
            |this, value, cx| this.set_setting(Setting::ComposeRich(value == "rich"), cx),
        );
        div()
            .flex()
            .flex_col()
            .child(page_head(
                Section::General.name(),
                Some(tr!("How Komorebi looks and starts.").into()),
                cx,
            ))
            .child(group_head(tr!("Appearance"), cx))
            .child(theme.icon(Icon::Color))
            .child(language.icon(Icon::Language))
            .child(check.icon(Icon::Mail))
            .child(undo.icon(Icon::Undo))
            .child(read.icon(Icon::MailRead))
            .child(group_head(tr!("Notifications"), cx))
            .child(flash)
            .child(notify.icon(Icon::Alert))
            .child(detail.icon(Icon::Eye))
            .child(group_head(tr!("Startup"), cx))
            .child(tray.icon(Icon::ToTray))
            .child(autostart.icon(Icon::Power))
            .child(tabs.icon(Icon::Tabs))
            .child(group_head(tr!("Compose"), cx))
            .child(format.icon(Icon::Compose))
            .child(self.font_card(cx))
    }
}

use super::Section;
