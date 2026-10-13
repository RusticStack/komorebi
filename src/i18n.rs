//! The display language: English (the source language) or European
//! Portuguese (Portugal; never Brazilian). It follows the Windows display
//! language unless Settings says otherwise.
//!
//! The text itself lives in the `.slint` files (`@tr`), bundled at build
//! time from `translations/pt-PT/LC_MESSAGES/komorebi.po`; `select` switches
//! which catalog Slint reads, and every `@tr` binding follows at once.

/// A language the app is shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    En,
    PtPt,
}

impl Lang {
    /// The name of its catalog folder under `translations/` (what
    /// `slint::select_bundled_translation` takes; `en` is the source text).
    pub fn tag(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::PtPt => "pt-PT",
        }
    }
}

/// The language choice in Settings (key `language`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LangSetting {
    /// Follow the Windows display language.
    #[default]
    System,
    English,
    PortuguesePortugal,
}

impl LangSetting {
    /// Its value in the settings store.
    pub fn key(self) -> &'static str {
        match self {
            LangSetting::System => "system",
            LangSetting::English => "en",
            LangSetting::PortuguesePortugal => "pt-PT",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key.trim() {
            "system" => Some(LangSetting::System),
            "en" => Some(LangSetting::English),
            "pt-PT" => Some(LangSetting::PortuguesePortugal),
            _ => None,
        }
    }

    /// Its place among the choices Settings lists (System, English,
    /// Português (Portugal)).
    pub fn index(self) -> i32 {
        match self {
            LangSetting::System => 0,
            LangSetting::English => 1,
            LangSetting::PortuguesePortugal => 2,
        }
    }

    /// The choice at `index` (out of range: the nearest end).
    pub fn from_index(index: i32) -> Self {
        match index {
            i if i <= 0 => LangSetting::System,
            1 => LangSetting::English,
            _ => LangSetting::PortuguesePortugal,
        }
    }
}

/// The language a Windows display-language tag asks for, if the app has
/// it: Portuguese other than Brazilian (`pt`, `pt-PT`, `pt-AO`, ...) is
/// European Portuguese, English is English; the rest, and `pt-BR`, are not
/// supported (`None`).
fn supported(tag: &str) -> Option<Lang> {
    let mut parts = tag.split(['-', '_']).map(str::to_ascii_lowercase);
    match parts.next()?.as_str() {
        "pt" => {
            let brazilian = parts.any(|p| p == "br");
            (!brazilian).then_some(Lang::PtPt)
        }
        "en" => Some(Lang::En),
        _ => None,
    }
}

/// The language to show: the choice if it names one; under System the first
/// supported entry of Windows' ordered display languages, else English.
pub fn resolve(setting: LangSetting, ui_languages: &[String]) -> Lang {
    match setting {
        LangSetting::English => Lang::En,
        LangSetting::PortuguesePortugal => Lang::PtPt,
        LangSetting::System => ui_languages
            .iter()
            .find_map(|tag| supported(tag))
            .unwrap_or(Lang::En),
    }
}

/// The language choice from the development switch `KOMOREBI_LANG`
/// (`system`, `en` or `pt-PT`), over the saved one for this run: for
/// snapshots and tests of a throwaway cache.
pub fn dev_setting() -> Option<LangSetting> {
    crate::dev_var("KOMOREBI_LANG")
        .and_then(|v| v.into_string().ok())
        .and_then(|v| LangSetting::from_key(&v))
}

/// Windows' display languages for this user, most preferred first, as
/// BCP 47 tags (`pt-PT`, `en-US`). Empty if Windows can't say. Only the
/// display language counts, not the regional format.
#[cfg(windows)]
pub fn os_ui_languages() -> Vec<String> {
    use windows_sys::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};
    let mut count = 0u32;
    let mut len = 0u32;
    // SAFETY: the first call only asks for the size; null is allowed then.
    let sized = unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            std::ptr::null_mut(),
            &mut len,
        )
    };
    if sized == 0 || len == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u16; len as usize];
    // SAFETY: `buf` holds `len` UTF-16 units, as the first call asked.
    let read = unsafe {
        GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, buf.as_mut_ptr(), &mut len)
    };
    if read == 0 {
        return Vec::new();
    }
    split_multi_string(&buf[..(len as usize).min(buf.len())])
}

#[cfg(not(windows))]
pub fn os_ui_languages() -> Vec<String> {
    Vec::new()
}

/// The strings of a Windows multi-string (each ends with a NUL, the list
/// with an empty one).
fn split_multi_string(units: &[u16]) -> Vec<String> {
    units
        .split(|&u| u == 0)
        .take_while(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

/// The language under `setting` on this computer.
pub fn resolve_here(setting: LangSetting) -> Lang {
    let languages = if setting == LangSetting::System {
        os_ui_languages()
    } else {
        Vec::new()
    };
    resolve(setting, &languages)
}

/// The language before the settings are read (the start-up failure dialog,
/// the first frame): Windows' display language, or the development switch.
pub fn startup_lang() -> Lang {
    resolve_here(dev_setting().unwrap_or_default())
}

/// Show the UI in `lang`: every `@tr` binding re-evaluates. Needs a window
/// to exist (Slint bundles its catalogs when the first one is created). On
/// failure the UI stays as it was; the result says whether it worked.
pub fn select(lang: Lang) -> bool {
    slint::select_bundled_translation(lang.tag()).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn system_follows_the_first_supported_display_language() {
        let sys = |list: &[&str]| resolve(LangSetting::System, &tags(list));
        assert_eq!(sys(&["pt-PT"]), Lang::PtPt);
        assert_eq!(sys(&["pt-PT", "en-US"]), Lang::PtPt);
        assert_eq!(sys(&["pt"]), Lang::PtPt, "bare pt is European");
        assert_eq!(sys(&["pt-AO"]), Lang::PtPt);
        assert_eq!(sys(&["PT-pt"]), Lang::PtPt, "case does not matter");
        assert_eq!(sys(&["pt_PT"]), Lang::PtPt);
        assert_eq!(sys(&["pt-Latn-PT"]), Lang::PtPt);
        assert_eq!(sys(&["en-US"]), Lang::En);
        assert_eq!(sys(&["en-GB", "pt-PT"]), Lang::En, "order counts");
        assert_eq!(sys(&[]), Lang::En);
        assert_eq!(sys(&["fr-FR"]), Lang::En);
        assert_eq!(
            sys(&["fr-FR", "pt-PT"]),
            Lang::PtPt,
            "unsupported is skipped"
        );
    }

    #[test]
    fn brazilian_portuguese_is_never_portugal() {
        let sys = |list: &[&str]| resolve(LangSetting::System, &tags(list));
        assert_eq!(sys(&["pt-BR"]), Lang::En);
        assert_eq!(sys(&["pt-Latn-BR"]), Lang::En);
        assert_eq!(sys(&["pt-br", "fr-FR"]), Lang::En);
        // Skipped, then a later supported entry wins.
        assert_eq!(sys(&["pt-BR", "pt-PT"]), Lang::PtPt);
        assert_eq!(sys(&["pt-BR", "en-US"]), Lang::En);
    }

    #[test]
    fn a_chosen_language_ignores_windows() {
        let pt = tags(&["pt-PT"]);
        let en = tags(&["en-US"]);
        assert_eq!(resolve(LangSetting::English, &pt), Lang::En);
        assert_eq!(resolve(LangSetting::PortuguesePortugal, &en), Lang::PtPt);
        assert_eq!(resolve(LangSetting::PortuguesePortugal, &[]), Lang::PtPt);
    }

    #[test]
    fn settings_round_trip() {
        for s in [
            LangSetting::System,
            LangSetting::English,
            LangSetting::PortuguesePortugal,
        ] {
            assert_eq!(LangSetting::from_key(s.key()), Some(s));
            assert_eq!(LangSetting::from_index(s.index()), s);
        }
        assert_eq!(
            LangSetting::from_key(" pt-PT\n"),
            Some(LangSetting::PortuguesePortugal)
        );
        assert_eq!(LangSetting::from_key("pt-BR"), None);
        assert_eq!(LangSetting::from_key(""), None);
        assert_eq!(LangSetting::from_index(-3), LangSetting::System);
        assert_eq!(LangSetting::from_index(99), LangSetting::PortuguesePortugal);
        assert_eq!(Lang::En.tag(), "en");
        assert_eq!(Lang::PtPt.tag(), "pt-PT");
    }

    #[test]
    fn multi_strings_split_at_nuls() {
        let wide = |s: &str| -> Vec<u16> { s.encode_utf16().collect() };
        let mut buf = wide("pt-PT");
        buf.push(0);
        buf.extend(wide("en-US"));
        buf.extend([0, 0]);
        assert_eq!(split_multi_string(&buf), ["pt-PT", "en-US"]);
        assert!(split_multi_string(&[0, 0]).is_empty());
        assert!(split_multi_string(&[]).is_empty());
        // No final terminator: still read.
        assert_eq!(split_multi_string(&wide("pt-PT")), ["pt-PT"]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_reports_display_languages() {
        // Whatever this machine uses, the call must work and give tags.
        let languages = os_ui_languages();
        assert!(!languages.is_empty());
        assert!(languages.iter().all(|t| t.len() >= 2 && !t.contains('\0')));
    }
}
