//! Settings > About (the Slint app's About section): the version, the
//! updates (the shell's updater view, mounted with
//! [`SettingsView::set_updates`]) with the automatic check's switch, Help
//! (What's new, Show tips again, Keyboard shortcuts, Get help, Report a
//! problem) and About (the privacy statement, the data folder, the source).
//! Every web page goes through the shell's link prompt
//! ([`SettingsEvent::OpenLink`]); nothing here touches the network.

use gpui::{ClipboardItem, Context, IntoElement, ParentElement, Styled, div, px};
use komorebi_i18n::tr;

use super::card::{Card, group_head, page_head};
use super::{Section, Setting, SettingsEvent, SettingsView, shortcuts};
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, space};
use crate::widgets::button::{Button, Variant};
use crate::widgets::toggle::Toggle;

/// The repository (the crate's `repository`).
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// The version shown.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The pages About opens (the Slint app's `about-link` numbers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Privacy,
    Changelog,
    Guide,
    Issues,
    Repository,
}

impl Page {
    /// The page's address.
    pub fn url(self) -> String {
        match self {
            Page::Privacy => format!("{REPOSITORY}/blob/main/PRIVACY.md"),
            Page::Changelog => format!("{REPOSITORY}/blob/main/CHANGELOG.md"),
            Page::Guide => format!("{REPOSITORY}#readme"),
            Page::Issues => format!("{REPOSITORY}/issues"),
            Page::Repository => REPOSITORY.to_string(),
        }
    }
}

/// The repository as the link shows it (no scheme).
pub fn repository_text() -> &'static str {
    REPOSITORY.strip_prefix("https://").unwrap_or(REPOSITORY)
}

impl SettingsView {
    /// The data folder (the cache's folder), as a path to show.
    pub fn data_folder(&self) -> Option<std::path::PathBuf> {
        self.db
            .file()
            .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
    }

    #[allow(clippy::too_many_arguments)] // a card's words, its page and its view
    fn link_card(
        &self,
        id: &'static str,
        icon: Icon,
        title: String,
        note: String,
        label: String,
        name: String,
        page: Page,
        cx: &mut Context<Self>,
    ) -> Card {
        Card::new(id, title).icon(icon).note(note).control(
            Button::new(id)
                .label(label)
                .name(name)
                .variant(Variant::OnInset)
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(SettingsEvent::OpenLink(page.url()));
                })),
        )
    }

    pub(crate) fn render_about(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let c = t.color;
        let this = cx.entity().downgrade();
        let version = Card::new("about-version", "Komorebi")
            .icon(Icon::Mail)
            .note(tr!("Version {0} · GPL-3.0-or-later", VERSION));
        let auto = Card::new("about-update-auto", tr!("Check for updates automatically"))
            .icon(Icon::Sync)
            .note(tr!("Once a day, a few seconds after Komorebi starts. Updates are never installed without your press."))
            .control(
                Toggle::new(
                    "about-update-auto-toggle",
                    tr!("Check for updates automatically"),
                    self.s.update_auto.as_deref() != Some("0"),
                )
                .state_label(tr!("On"), tr!("Off"))
                .on_change({
                    let this = this.clone();
                    move |on, _, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.set_setting(Setting::UpdateAuto(on), cx)
                        });
                    }
                }),
            );
        let news = self.link_card(
            "about-news",
            Icon::Info,
            tr!("What's new in {0}", VERSION),
            tr!("The changes in this version, in the changelog."),
            tr!("about" => "Open"),
            tr!("Open the changelog in the browser"),
            Page::Changelog,
            cx,
        );
        let tips = Card::new("about-tips", tr!("Show tips again"))
            .icon(Icon::Info)
            .note(tr!(
                "Shows the short tips about folders, search and help again."
            ))
            .control(
                Button::new("about-tips-button")
                    .label(tr!("Show tips"))
                    .variant(Variant::OnInset)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::ShowTips))),
            );
        let keys = Card::new("about-keys", tr!("Keyboard shortcuts"))
            .icon(Icon::Keyboard)
            .note(tr!("Every key, by group. Press F1 or Ctrl+/ anywhere."))
            .control(
                Button::new("about-keys-button")
                    .label(tr!("about" => "Show"))
                    .name(tr!("Show the keyboard shortcuts"))
                    .variant(Variant::OnInset)
                    .on_click(|_, window, cx| shortcuts::open(window, cx)),
            );
        let help = self.link_card(
            "about-help",
            Icon::Info,
            tr!("Get help"),
            tr!("The Komorebi guide: adding an account, the keys and the settings."),
            tr!("about" => "Open"),
            tr!("Open the Komorebi guide in the browser"),
            Page::Guide,
            cx,
        );
        let report = self.link_card(
            "about-report",
            Icon::Flag,
            tr!("Report a problem"),
            tr!("Opens the issues page in the browser. No data is attached."),
            tr!("Report"),
            tr!("Open the issues page in the browser"),
            Page::Issues,
            cx,
        );
        let privacy = self.link_card(
            "about-privacy",
            Icon::Document,
            tr!("Privacy statement"),
            tr!("What Komorebi keeps on this PC, and what it sends where."),
            tr!("about" => "Open"),
            tr!("Open the privacy statement in the browser"),
            Page::Privacy,
            cx,
        );
        let folder = self.data_folder();
        let shown = folder
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let data = Card::new("about-data", tr!("Data folder"))
            .icon(Icon::Folder)
            .note(shown.clone())
            .control(
                Button::new("about-data-copy")
                    .label(tr!("Copy"))
                    .name(tr!("Copy the data folder's path"))
                    .variant(Variant::OnInset)
                    .disabled(shown.is_empty())
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(shown.clone()))
                    }),
            )
            .control(
                Button::new("about-data-open")
                    .label(tr!("about" => "Open"))
                    .name(tr!("Open the data folder in File Explorer"))
                    .variant(Variant::OnInset)
                    .disabled(folder.is_none())
                    .on_click(move |_, _, cx| {
                        if let Some(dir) = &folder {
                            cx.open_with_system(dir);
                        }
                    }),
            );
        let source = Card::new("about-source", tr!("Source code"))
            .icon(Icon::Link)
            .note(tr!("Free and open source, with no telemetry and no ads. Passwords are kept in Windows Credential Manager."))
            .control(
                Button::new("about-source-link")
                    .label(repository_text())
                    .name(tr!("Open the Komorebi repository in the browser"))
                    .variant(Variant::Subtle)
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(SettingsEvent::OpenLink(Page::Repository.url()));
                    })),
            );
        div()
            .flex()
            .flex_col()
            .text_color(c.ink)
            .child(page_head(Section::About.name(), None, cx))
            .child(div().mt(px(space::SP_6)).child(version))
            .child(group_head(tr!("about group" => "Updates"), cx))
            .children(
                self.updates
                    .clone()
                    .map(|u| div().mb(px(space::SP_1)).child(u)),
            )
            .child(auto)
            .child(group_head(tr!("about group" => "Help"), cx))
            .child(news)
            .child(tips)
            .child(keys)
            .child(help)
            .child(report)
            .child(group_head(tr!("about group" => "About"), cx))
            .child(privacy)
            .child(data)
            .child(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The app and the library carry one version: About shows this one, the
    /// update check compares the library's, and a pre-release such as
    /// `1.0.0-alpha.0` is no version to the check (it would read 0.0.0).
    #[test]
    fn the_app_and_the_library_have_one_release_version() {
        assert_eq!(VERSION, komorebi::update::current().to_string());
        assert_ne!(VERSION, "0.0.0");
    }

    #[test]
    fn the_pages_are_the_repository_s() {
        assert_eq!(repository_text(), "github.com/RusticStack/komorebi");
        assert!(Page::Privacy.url().ends_with("/blob/main/PRIVACY.md"));
        assert!(Page::Changelog.url().ends_with("/blob/main/CHANGELOG.md"));
        assert!(Page::Guide.url().ends_with("#readme"));
        assert!(Page::Issues.url().ends_with("/issues"));
    }
}
