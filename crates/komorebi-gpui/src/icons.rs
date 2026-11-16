//! Icons (BRAND.md "Iconography", docs/FLUENT2.md "Iconography"): Fluent
//! System Icons (MIT, `@fluentui/svg-icons` 1.1.292), bundled as SVG and
//! drawn as a mask in the caller's text colour. They are the same shapes as
//! the Segoe Fluent Icons font, which Windows 10 does not have; bundling
//! them means one look on both, with no font fallback. The brand's own
//! glyph is the seal (`Seal`), from komorebi-brand `logo/mark-mono.svg`.
//!
//! Fluent draws each icon once per size, with strokes tuned to that size
//! (1 px at every size, so a 16 px glyph scaled to 20 px is a blurred,
//! lighter one, and a 20 px glyph at 16 px a thinner one). So every icon
//! comes in the sizes the app draws it at, and [`Icon::at`] picks the set
//! made for a size:
//!
//! - `icons/fluent/<name>.svg`: 16 px regular (`<name>_16_regular`; Power
//!   only has 20), for dense places: menus, small buttons, row marks.
//! - `icons/fluent/20/<name>.svg`: 20 px regular (`<name>_20_regular`), the
//!   standard size beside 14 px text (navigation, command bars, buttons).
//! - `icons/fluent/20/<name>_filled.svg`: 20 px filled (`<name>_20_filled`),
//!   for the selected or on state of the icons that have one ([`Icon::filled`]).
//! - `icons/fluent/12/<name>.svg`: 12 px regular (`<name>_12_regular`), for
//!   the small chevrons and the tab's close glyph.
//!
//! [`AppAssets`] serves these ahead of GPUI Kit's default `Assets` (the
//! about 100 Lucide icons Kit's own components draw; not `AllAssets`, to
//! keep the exe small). Own paths live under `komorebi/` so they
//! never shadow a Kit icon (cookbook_assets).

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};
use gpui_kit::component::IconNamed;

use crate::theme::layout;

/// The directory of every Fluent icon file, under `assets/`.
macro_rules! fluent {
    ($name:literal) => {
        concat!("icons/fluent/", $name, ".svg")
    };
    (20, $name:literal) => {
        concat!("icons/fluent/20/", $name, ".svg")
    };
    (20f, $name:literal) => {
        concat!("icons/fluent/20/", $name, "_filled.svg")
    };
    (12, $name:literal) => {
        concat!("icons/fluent/12/", $name, ".svg")
    };
}

macro_rules! file {
    ($path:expr) => {
        (
            concat!("komorebi/", $path),
            include_bytes!(concat!("../assets/", $path)) as &[u8],
        )
    };
}

macro_rules! icons {
    ($($variant:ident => $name:literal $(, filled $f:tt)? $(, small $s:tt)?;)*) => {
        /// Every icon the app draws.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Icon {
            Seal,
            $($variant,)*
        }

        impl Icon {
            pub const ALL: &[Icon] = &[Icon::Seal, $(Icon::$variant,)*];

            /// The 16 px regular glyph (the seal: its one file).
            fn regular16(self) -> (&'static str, &'static [u8]) {
                match self {
                    Icon::Seal => file!("brand/seal.svg"),
                    $(Icon::$variant => file!(fluent!($name)),)*
                }
            }

            /// The 20 px regular glyph.
            fn regular20(self) -> Option<(&'static str, &'static [u8])> {
                match self {
                    Icon::Seal => None,
                    $(Icon::$variant => Some(file!(fluent!(20, $name))),)*
                }
            }

            /// The 20 px filled glyph, for the icons that have one.
            fn filled20(self) -> Option<(&'static str, &'static [u8])> {
                match self {
                    $($(Icon::$variant => { let _ = $f; Some(file!(fluent!(20f, $name))) })?)*
                    _ => None,
                }
            }

            /// The 12 px regular glyph, for the icons that have one.
            fn regular12(self) -> Option<(&'static str, &'static [u8])> {
                match self {
                    $($(Icon::$variant => { let _ = $s; Some(file!(fluent!(12, $name))) })?)*
                    _ => None,
                }
            }
        }
    };
}

icons! {
    Add => "add";
    Alert => "alert";
    Archive => "archive", filled 1;
    Attach => "attach";
    Calendar => "calendar_ltr";
    Checkmark => "checkmark";
    ChevronDown => "chevron_down", small 1;
    ChevronLeft => "chevron_left", small 1;
    ChevronRight => "chevron_right", small 1;
    ChevronUp => "chevron_up", small 1;
    Color => "color";
    Compose => "compose", filled 1;
    Delete => "delete", filled 1;
    Dismiss => "dismiss", small 1;
    Document => "document", filled 1;
    ErrorCircle => "error_circle";
    Filter => "filter";
    Flag => "flag", filled 1;
    Folder => "folder", filled 1;
    // Move to a folder (the Move command and menu button).
    MoveTo => "folder_arrow_right";
    Forward => "arrow_forward";
    Image => "image";
    Info => "info";
    Language => "local_language";
    Mail => "mail", filled 1;
    MailRead => "mail_read";
    Maximize => "maximize";
    More => "more_horizontal";
    Navigation => "navigation";
    Open => "open";
    People => "people", filled 1;
    Person => "person", filled 1;
    Pin => "pin", filled 1;
    Power => "power";
    Reply => "arrow_reply";
    ReplyAll => "arrow_reply_all";
    Restore => "square_multiple";
    // Save to a file (an attachment's Save, Save as .eml).
    Save => "save";
    Search => "search";
    Send => "send", filled 1;
    Settings => "settings", filled 1;
    Minimize => "subtract";
    Sync => "arrow_sync";
    TextSize => "text_font";
    Warning => "warning", filled 1;
    // The composer's format bar and menus (phase 4).
    Bold => "text_bold";
    Italic => "text_italic";
    Underline => "text_underline";
    Strikethrough => "text_strikethrough";
    Bullets => "text_bullet_list_ltr";
    Numbers => "text_number_list_ltr";
    Link => "link";
    QuoteText => "text_quote";
    IndentMore => "text_indent_increase_ltr";
    IndentLess => "text_indent_decrease_ltr";
    ClearFormat => "text_clear_formatting";
    // The format bar's alignment box and font colour (1.0.3: Gmail's
    // order). Justify has no 16 px drawing in 1.1.292: its 16 px file is
    // the 20 px one, as Power's is.
    AlignLeft => "text_align_left";
    AlignCenter => "text_align_center";
    AlignRight => "text_align_right";
    AlignJustify => "text_align_justify";
    TextColor => "text_color";
    Table => "table";
    Important => "important";
    ArrowDown => "arrow_down";
    Signature => "signature";
    Eye => "eye", filled 1;
    Edit => "edit";
    Copy => "copy";
    PersonAdd => "person_add";
    // A rule card's drag handle (stage E, data).
    ReOrder => "re_order_dots_vertical";
    // Settings > General's cards (1.0.2: every card of a page has its icon,
    // so the titles make one left edge).
    Undo => "arrow_undo";
    Flash => "flash";
    Tabs => "tabs";
    ToTray => "arrow_minimize";
    Keyboard => "keyboard";
}

impl Icon {
    /// The 16 px regular glyph's asset path (dense places; see [`Icon::at`]).
    pub fn path(self) -> &'static str {
        self.regular16().0
    }

    /// The glyph drawn for `size` logical px: the 20 px set from 18 px up,
    /// the 12 px set (where there is one) up to 13 px, else the 16 px set.
    pub fn at(self, size: f32) -> &'static str {
        self.file_at(size, false).0
    }

    /// The filled glyph for `size` (the selected or on state), or the
    /// regular one where Fluent's filled set is not bundled.
    pub fn filled(self, size: f32) -> &'static str {
        self.file_at(size, true).0
    }

    /// The glyph for `size`, filled when `on`.
    pub fn state(self, size: f32, on: bool) -> &'static str {
        self.file_at(size, on).0
    }

    fn file_at(self, size: f32, filled: bool) -> (&'static str, &'static [u8]) {
        if size >= layout::ICON_LG - 2. {
            let pick = if filled { self.filled20() } else { None };
            if let Some(file) = pick.or_else(|| self.regular20()) {
                return file;
            }
        }
        if size <= layout::ICON_SM + 1.
            && let Some(file) = self.regular12()
        {
            return file;
        }
        self.regular16()
    }

    /// Every bundled file of this icon (path, bytes).
    fn files(self) -> impl Iterator<Item = (&'static str, &'static [u8])> {
        [
            Some(self.regular16()),
            self.regular20(),
            self.filled20(),
            self.regular12(),
        ]
        .into_iter()
        .flatten()
    }

    fn find(path: &str) -> Option<&'static [u8]> {
        Icon::ALL
            .iter()
            .flat_map(|i| i.files())
            .find(|(p, _)| *p == path)
            .map(|(_, bytes)| bytes)
    }
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
        if let Some(bytes) = Icon::find(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
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
                .flat_map(|i| i.files())
                .filter(|(p, _)| p.starts_with(path))
                .map(|(p, _)| SharedString::from(p)),
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
            for (path, _) in icon.files() {
                let bytes = AppAssets.load(path).expect("load").expect("present");
                assert!(bytes.starts_with(b"<svg"), "{icon:?} {path}");
                assert!(bytes.len() < 4096, "{icon:?} {path}: {} bytes", bytes.len());
            }
        }
    }

    /// Each size gets the glyph drawn for it: 20 px for the standard size,
    /// 16 px for dense places, 12 px for the small chevrons; filled only
    /// where Fluent's filled glyph is bundled.
    #[test]
    fn the_glyph_follows_the_size_and_state() {
        let view_box = |path: &str| {
            let bytes = AppAssets.load(path).expect("load").expect("present");
            let text = String::from_utf8(bytes.to_vec()).expect("utf-8");
            text.split("viewBox=\"0 0 ")
                .nth(1)
                .and_then(|s| s.split(' ').next())
                .and_then(|s| s.parse::<u32>().ok())
                .expect("a view box")
        };
        assert_eq!(view_box(Icon::Mail.at(layout::ICON_LG)), 20);
        assert_eq!(view_box(Icon::Mail.at(layout::ICON)), 16);
        assert_eq!(view_box(Icon::ChevronDown.at(layout::ICON_SM)), 12);
        assert_eq!(
            view_box(Icon::Mail.at(layout::ICON_SM)),
            16,
            "no 12 px mail"
        );
        assert!(
            Icon::Mail
                .filled(layout::ICON_LG)
                .ends_with("mail_filled.svg")
        );
        assert_eq!(Icon::Mail.filled(layout::ICON), Icon::Mail.path());
        assert_eq!(
            Icon::Bold.filled(layout::ICON_LG),
            Icon::Bold.at(layout::ICON_LG),
            "no filled bold"
        );
        assert_eq!(Icon::Seal.at(layout::ICON_LG), Icon::Seal.path());
    }

    /// NOTICE says how many Fluent icons are bundled, and from which
    /// package version: the folder must agree (every file there is one the
    /// notice counts). The files themselves were compared byte for byte
    /// (white space aside) with `@fluentui/svg-icons` 1.1.292 in stage D.
    #[test]
    fn notice_counts_the_bundled_fluent_icons() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = 0;
        let mut dirs = vec![root.join("assets/icons/fluent")];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir).expect("the icon folder").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|x| x == "svg") {
                    files += 1;
                }
            }
        }
        // Every file there is one an icon serves (the seal is not Fluent's).
        let served: usize = Icon::ALL.iter().map(|i| i.files().count()).sum();
        assert_eq!(files, served - 1);
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
        let icons: usize = Icon::ALL.iter().map(|i| i.files().count()).sum();
        assert_eq!(own.len(), icons + providers);
        let gmail = AppAssets
            .load("komorebi/providers/gmail@2x.png")
            .expect("load")
            .expect("present");
        assert!(gmail.starts_with(b"\x89PNG"));
    }
}
