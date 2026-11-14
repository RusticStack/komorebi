//! Icons (BRAND.md "Iconography", IMPLEMENTATION.md `icons.rs`): Fluent
//! System Icons (MIT, `@fluentui/svg-icons` 1.1.292), regular weight, 16 px (20 px for Power),
//! bundled as SVG and drawn as a mask in the caller's text colour. They are
//! the same shapes as the Segoe Fluent Icons font, which Windows 10 does not
//! have; bundling them means one look on both, with no font fallback
//! (the Slint app falls back to Segoe MDL2 Assets on Windows 10). The
//! brand's own glyph is the seal (`Seal`), from komorebi-brand
//! `logo/mark-mono.svg`.
//!
//! [`AppAssets`] serves these ahead of GPUI Kit's default `Assets` (the
//! about 100 Lucide icons Kit's own components draw; not `AllAssets`, to
//! keep the exe small). Own paths live under `komorebi/` so they
//! never shadow a Kit icon (cookbook_assets).

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};
use gpui_kit::component::IconNamed;

macro_rules! icons {
    ($($variant:ident => $file:literal,)*) => {
        /// Every icon the app draws.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Icon {
            $($variant,)*
        }

        impl Icon {
            pub const ALL: &[Icon] = &[$(Icon::$variant,)*];

            /// The asset path.
            pub fn path(self) -> &'static str {
                match self {
                    $(Icon::$variant => concat!("komorebi/", $file),)*
                }
            }

            fn bytes(self) -> &'static [u8] {
                match self {
                    $(Icon::$variant => include_bytes!(concat!("../assets/", $file)),)*
                }
            }
        }
    };
}

icons! {
    Seal => "brand/seal.svg",
    Add => "icons/fluent/add.svg",
    Alert => "icons/fluent/alert.svg",
    Archive => "icons/fluent/archive.svg",
    Attach => "icons/fluent/attach.svg",
    Calendar => "icons/fluent/calendar_ltr.svg",
    Checkmark => "icons/fluent/checkmark.svg",
    ChevronDown => "icons/fluent/chevron_down.svg",
    ChevronLeft => "icons/fluent/chevron_left.svg",
    ChevronRight => "icons/fluent/chevron_right.svg",
    ChevronUp => "icons/fluent/chevron_up.svg",
    Color => "icons/fluent/color.svg",
    Compose => "icons/fluent/compose.svg",
    Delete => "icons/fluent/delete.svg",
    Dismiss => "icons/fluent/dismiss.svg",
    Document => "icons/fluent/document.svg",
    ErrorCircle => "icons/fluent/error_circle.svg",
    Filter => "icons/fluent/filter.svg",
    Flag => "icons/fluent/flag.svg",
    Folder => "icons/fluent/folder.svg",
    // Move to a folder (the Move command and menu button).
    MoveTo => "icons/fluent/folder_arrow_right.svg",
    Forward => "icons/fluent/arrow_forward.svg",
    Image => "icons/fluent/image.svg",
    Info => "icons/fluent/info.svg",
    Language => "icons/fluent/local_language.svg",
    Mail => "icons/fluent/mail.svg",
    MailRead => "icons/fluent/mail_read.svg",
    Maximize => "icons/fluent/maximize.svg",
    More => "icons/fluent/more_horizontal.svg",
    Navigation => "icons/fluent/navigation.svg",
    Open => "icons/fluent/open.svg",
    People => "icons/fluent/people.svg",
    Person => "icons/fluent/person.svg",
    Pin => "icons/fluent/pin.svg",
    Power => "icons/fluent/power.svg",
    Reply => "icons/fluent/arrow_reply.svg",
    ReplyAll => "icons/fluent/arrow_reply_all.svg",
    Restore => "icons/fluent/square_multiple.svg",
    // Save to a file (an attachment's Save, Save as .eml).
    Save => "icons/fluent/save.svg",
    Search => "icons/fluent/search.svg",
    Send => "icons/fluent/send.svg",
    Settings => "icons/fluent/settings.svg",
    Minimize => "icons/fluent/subtract.svg",
    Sync => "icons/fluent/arrow_sync.svg",
    TextSize => "icons/fluent/text_font.svg",
    Warning => "icons/fluent/warning.svg",
    // The composer's format bar and menus (phase 4).
    Bold => "icons/fluent/text_bold.svg",
    Italic => "icons/fluent/text_italic.svg",
    Underline => "icons/fluent/text_underline.svg",
    Strikethrough => "icons/fluent/text_strikethrough.svg",
    Bullets => "icons/fluent/text_bullet_list_ltr.svg",
    Numbers => "icons/fluent/text_number_list_ltr.svg",
    Link => "icons/fluent/link.svg",
    QuoteText => "icons/fluent/text_quote.svg",
    IndentMore => "icons/fluent/text_indent_increase_ltr.svg",
    IndentLess => "icons/fluent/text_indent_decrease_ltr.svg",
    ClearFormat => "icons/fluent/text_clear_formatting.svg",
    Table => "icons/fluent/table.svg",
    Important => "icons/fluent/important.svg",
    ArrowDown => "icons/fluent/arrow_down.svg",
    Signature => "icons/fluent/signature.svg",
    Eye => "icons/fluent/eye.svg",
    // Rules and Contacts (phases 8 and 9; the 20 px glyphs, drawn at 16).
    Edit => "icons/fluent/edit.svg",
    Copy => "icons/fluent/copy.svg",
    PersonAdd => "icons/fluent/person_add.svg",
    // A rule card's drag handle (stage E, data).
    ReOrder => "icons/fluent/re_order_dots_vertical.svg",
}

impl IconNamed for Icon {
    fn path(self) -> SharedString {
        Icon::path(self).into()
    }
}

/// The app's icons first, then Kit's default bundle.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(icon) = Icon::ALL.iter().find(|i| i.path() == path) {
            return Ok(Some(Cow::Borrowed(icon.bytes())));
        }
        // The providers' own icons (the setup's provider list), compiled in.
        if let Some(bytes) = crate::views::onboarding::providers::icon_bytes(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut list = gpui_kit::assets::Assets.list(path)?;
        list.extend(
            Icon::ALL
                .iter()
                .filter(|i| i.path().starts_with(path))
                .map(|i| SharedString::from(i.path())),
        );
        list.extend(
            crate::views::onboarding::providers::icon_paths().filter(|p| p.starts_with(path)),
        );
        Ok(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_small_svg() {
        for icon in Icon::ALL {
            let bytes = AppAssets.load(icon.path()).expect("load").expect("present");
            assert!(bytes.starts_with(b"<svg"), "{icon:?}");
            assert!(bytes.len() < 4096, "{icon:?}: {} bytes", bytes.len());
        }
    }

    /// NOTICE says how many Fluent icons are bundled, and from which
    /// package version: the folder must agree (every file there is one the
    /// notice counts). The files themselves were compared byte for byte
    /// (white space aside) with `@fluentui/svg-icons` 1.1.292 in stage D.
    #[test]
    fn notice_counts_the_bundled_fluent_icons() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let files = std::fs::read_dir(root.join("assets/icons/fluent"))
            .expect("the icon folder")
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "svg"))
            .count();
        let notice = std::fs::read_to_string(root.join("../../NOTICE")).expect("NOTICE");
        let text = notice.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            text.contains("@fluentui/svg-icons 1.1.292 package), ")
                && text.contains(&format!("{files} of them bundled")),
            "NOTICE and assets/icons/fluent disagree ({files} files)"
        );
    }

    #[test]
    fn kit_icons_still_load_and_own_paths_list() {
        let kit = AppAssets
            .load("icons/close.svg")
            .expect("kit")
            .expect("present");
        assert!(kit.starts_with(b"<svg"));
        let own = AppAssets.list("komorebi/").expect("list");
        let providers = crate::views::onboarding::providers::icon_paths().count();
        assert_eq!(own.len(), Icon::ALL.len() + providers);
        let gmail = AppAssets
            .load("komorebi/providers/gmail@2x.png")
            .expect("load")
            .expect("present");
        assert!(gmail.starts_with(b"\x89PNG"));
    }
}
