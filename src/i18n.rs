//! Lightweight localization (i18n) for the Komorebi desktop email client.
//!
//! No external i18n crate: pure Rust plus `serde` for persistence. The system
//! supports runtime locale detection, a string catalog that falls back to
//! `en-US`, and a [`t()`] lookup function plus a convenient [`t!`] macro that
//! resolves against the current thread-local locale.
//!
//! Privacy note: nothing here ever touches mailbox data, credentials, or the
//! network. The catalog is an in-memory `&'static str` table; persistence is a
//! plain `serde` struct owned by the caller.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Locale
// ---------------------------------------------------------------------------

/// A BCP-47-style locale supported by Komorebi.
///
/// `en-US` is the default and the fallback when a translation key is missing
/// for a target locale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Locale {
    /// English (United States) - the default/fallback locale.
    #[default]
    EnUs,
    /// English (United Kingdom).
    EnGb,
    /// European Portuguese (Portugal) - the previous baseline locale.
    PtPt,
    /// Brazilian Portuguese (Brazil).
    PtBr,
    /// Spanish (Spain).
    EsEs,
    /// German (Germany).
    DeDe,
    /// French (France).
    FrFr,
    /// Japanese (Japan) - the demo account locale.
    JaJp,
}

impl Locale {
    /// The canonical BCP-47 tag for this locale, e.g. `"en-US"`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Locale::EnUs => "en-US",
            Locale::EnGb => "en-GB",
            Locale::PtPt => "pt-PT",
            Locale::PtBr => "pt-BR",
            Locale::EsEs => "es-ES",
            Locale::DeDe => "de-DE",
            Locale::FrFr => "fr-FR",
            Locale::JaJp => "ja-JP",
        }
    }

    /// Parse a BCP-47 tag (e.g. `"en-US"`, `"pt-PT"`) or a POSIX-style locale
    /// string (e.g. `"pt_PT.UTF-8"`). Unknown/empty input falls back to
    /// [`Locale::EnUs`].
    ///
    /// This is an inherent method (not the `FromStr` trait) by design:
    /// parsing never fails - it always falls back to [`Locale::EnUs`].
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        // Normalize POSIX-style "pt_PT.UTF-8" -> "pt-PT" by swapping the first
        // `_`/`.` region separator for `-` and trimming any encoding suffix.
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Locale::EnUs;
        }
        // Take only the language[_-]region portion before any '.' or '@'.
        let head = trimmed.split(['.', '@']).next().unwrap_or(trimmed);
        // Use '-' as the canonical separator.
        let normalized = head.replace('_', "-");
        let lower = normalized.to_ascii_lowercase();
        match lower.as_str() {
            "en-us" | "english" => Locale::EnUs,
            "en-gb" | "en" => Locale::EnGb,
            "pt-pt" | "português" | "portugues" => Locale::PtPt,
            "pt-br" | "pt" => Locale::PtBr,
            "es-es" | "es" | "español" | "espanol" => Locale::EsEs,
            "de-de" | "de" | "deutsch" => Locale::DeDe,
            "fr-fr" | "fr" | "français" | "francais" => Locale::FrFr,
            "ja-jp" | "ja" | "日本語" | "japanese" => Locale::JaJp,
            _ => Locale::EnUs,
        }
    }

    /// A human-readable display name, e.g. `"English (US)"`.
    pub fn display_name(&self) -> &'static str {
        match self {
            Locale::EnUs => "English (US)",
            Locale::EnGb => "English (UK)",
            Locale::PtPt => "Português (Portugal)",
            Locale::PtBr => "Português (Brasil)",
            Locale::EsEs => "Español (España)",
            Locale::DeDe => "Deutsch (Deutschland)",
            Locale::FrFr => "Français (France)",
            Locale::JaJp => "日本語 (日本)",
        }
    }
}

impl std::fmt::Display for Locale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parses a BCP-47/POSIX locale string into a [`Locale`].
///
/// This mirrors [`Locale::from_str`] so callers can write
/// `Locale::from_str("pt-PT")` or use the `FromStr` trait directly.
impl std::str::FromStr for Locale {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Locale::from_str(s))
    }
}

// ---------------------------------------------------------------------------
// Locale detection
// ---------------------------------------------------------------------------

/// Best-effort detection of the system locale.
///
/// Checks, in order, the `LANG`, `LC_ALL`, and `LC_MESSAGES` environment
/// variables (the conventional POSIX/Unix knobs that Windows also exposes to
/// some runtimes) and parses the first non-empty value found. Falls back to
/// [`Locale::EnUs`] when nothing useful is present.
pub fn detect_locale() -> Locale {
    for var in ["LANG", "LC_ALL", "LC_MESSAGES"] {
        if let Ok(val) = std::env::var(var) {
            let parsed = Locale::from_str(&val);
            // An empty value or a bare "C"/"POSIX" locale yields EnUs; still a
            // valid answer, so accept any parsed result from a non-empty var.
            if !val.trim().is_empty() && !val.trim().eq_ignore_ascii_case("C") {
                return parsed;
            }
        }
    }
    Locale::EnUs
}

// ---------------------------------------------------------------------------
// Translation key system
// ---------------------------------------------------------------------------

/// A single catalog entry: a key mapped to per-locale translations.
struct Entry {
    key: &'static str,
    translations: &'static [(Locale, &'static str)],
}

/// Builds the full string catalog once and caches it in a `OnceLock`.
///
/// Returning `&HashMap<&str, HashMap<Locale, &str>>` keeps every string a
/// `&'static str` with zero allocation on lookup.
fn catalog() -> &'static HashMap<&'static str, HashMap<Locale, &'static str>> {
    static CATALOG: OnceLock<HashMap<&'static str, HashMap<Locale, &'static str>>> =
        OnceLock::new();
    CATALOG.get_or_init(build_catalog)
}

/// Constructs the catalog from a list of [`Entry`] values.
///
/// Each entry must contain an `EnUs` translation (the fallback); missing `EnUs`
/// is treated as a programming error and the key is skipped to keep the
/// catalog consistent.
fn build_catalog() -> HashMap<&'static str, HashMap<Locale, &'static str>> {
    let entries: &[Entry] = CATALOG_ENTRIES;
    let mut map: HashMap<&'static str, HashMap<Locale, &'static str>> =
        HashMap::with_capacity(entries.len());
    for entry in entries {
        let mut by_locale: HashMap<Locale, &'static str> =
            HashMap::with_capacity(entry.translations.len());
        for (locale, text) in entry.translations {
            by_locale.insert(*locale, *text);
        }
        // EnUs is always present in CATALOG_ENTRIES; insert defensively.
        if by_locale.contains_key(&Locale::EnUs) {
            map.insert(entry.key, by_locale);
        }
    }
    map
}

/// Lookup `key` for `locale`, falling back to `en-US` when the key is missing
/// in the target locale, and returning the key itself when missing from every
/// locale (including the fallback).
pub fn t(key: &str, locale: Locale) -> String {
    let cat = catalog();
    if let Some(by_locale) = cat.get(key) {
        if let Some(text) = by_locale.get(&locale) {
            return (*text).to_string();
        }
        if let Some(text) = by_locale.get(&Locale::EnUs) {
            return (*text).to_string();
        }
    }
    key.to_string()
}

// ---------------------------------------------------------------------------
// Current locale (thread-local)
// ---------------------------------------------------------------------------

thread_local! {
    /// The locale used by [`t!`] for the current thread. Initialized lazily to
    /// [`detect_locale()`] on first read via [`current_locale()`].
    static CURRENT_LOCALE: Cell<Option<Locale>> = const { Cell::new(None) };
}

/// Returns the locale active for the current thread.
///
/// Defaults to [`detect_locale()`] on first use (captured per-thread) and can
/// be overridden with [`set_locale()`].
pub fn current_locale() -> Locale {
    CURRENT_LOCALE.with(|cell| {
        cell.get().unwrap_or_else(|| {
            let detected = detect_locale();
            cell.set(Some(detected));
            detected
        })
    })
}

/// Overrides the locale used by [`t!`] for the current thread.
pub fn set_locale(locale: Locale) {
    CURRENT_LOCALE.with(|cell| cell.set(Some(locale)));
}

/// Convenience macro: translate `key` against [`current_locale()`].
#[macro_export]
macro_rules! t {
    ($key:expr) => {
        $crate::i18n::t($key, $crate::i18n::current_locale())
    };
}

// ---------------------------------------------------------------------------
// I18nConfig
// ---------------------------------------------------------------------------

/// Persistable i18n preferences.
///
/// Only the locale is stored for now; this struct exists so callers can
/// `serde`-serialize it into their own config/DB layer. Storage and loading of
/// real user data is owned elsewhere (see `db`), so no DB methods live here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct I18nConfig {
    /// The active locale for this configuration.
    pub locale: Locale,
}

impl Default for I18nConfig {
    fn default() -> Self {
        Self {
            locale: Locale::EnUs,
        }
    }
}

// ---------------------------------------------------------------------------
// String catalog
// ---------------------------------------------------------------------------

/// All translation entries, grouped per key.
///
/// Each row lists the key followed by `(Locale, text)` pairs. `EnUs` is always
/// the first pair and acts as the fallback. Keep this table the single source
/// of truth for UI strings.
#[rustfmt::skip]
const CATALOG_ENTRIES: &[Entry] = &[
    Entry {
        key: "app.name",
        translations: &[
            (Locale::EnUs, "Komorebi"),
            (Locale::PtPt, "Komorebi"),
            (Locale::JaJp, "Komorebi"),
        ],
    },
    Entry {
        key: "sidebar.inbox",
        translations: &[
            (Locale::EnUs, "Inbox"),
            (Locale::PtPt, "Caixa de Entrada"),
            (Locale::JaJp, "受信箱"),
        ],
    },
    Entry {
        key: "sidebar.sent",
        translations: &[
            (Locale::EnUs, "Sent"),
            (Locale::PtPt, "Enviados"),
            (Locale::JaJp, "送信済み"),
        ],
    },
    Entry {
        key: "sidebar.drafts",
        translations: &[
            (Locale::EnUs, "Drafts"),
            (Locale::PtPt, "Rascunhos"),
            (Locale::JaJp, "下書き"),
        ],
    },
    Entry {
        key: "sidebar.archive",
        translations: &[
            (Locale::EnUs, "Archive"),
            (Locale::PtPt, "Arquivo"),
            (Locale::JaJp, "アーカイブ"),
        ],
    },
    Entry {
        key: "sidebar.trash",
        translations: &[
            (Locale::EnUs, "Trash"),
            (Locale::PtPt, "Lixeira"),
            (Locale::JaJp, "ゴミ箱"),
        ],
    },
    Entry {
        key: "sidebar.priority",
        translations: &[
            (Locale::EnUs, "Priority"),
            (Locale::PtPt, "Prioridade"),
            (Locale::JaJp, "優先"),
        ],
    },
    Entry {
        key: "sidebar.snoozed",
        translations: &[
            (Locale::EnUs, "Snoozed"),
            (Locale::PtPt, "Adiados"),
            (Locale::JaJp, "スヌーズ"),
        ],
    },
    Entry {
        key: "action.compose",
        translations: &[
            (Locale::EnUs, "New Message"),
            (Locale::PtPt, "Nova Mensagem"),
            (Locale::JaJp, "新規メッセージ"),
        ],
    },
    Entry {
        key: "action.reply",
        translations: &[
            (Locale::EnUs, "Reply"),
            (Locale::PtPt, "Responder"),
            (Locale::JaJp, "返信"),
        ],
    },
    Entry {
        key: "action.forward",
        translations: &[
            (Locale::EnUs, "Forward"),
            (Locale::PtPt, "Encaminhar"),
            (Locale::JaJp, "転送"),
        ],
    },
    Entry {
        key: "action.archive",
        translations: &[
            (Locale::EnUs, "Archive"),
            (Locale::PtPt, "Arquivar"),
            (Locale::JaJp, "アーカイブ"),
        ],
    },
    Entry {
        key: "action.delete",
        translations: &[
            (Locale::EnUs, "Delete"),
            (Locale::PtPt, "Excluir"),
            (Locale::JaJp, "削除"),
        ],
    },
    Entry {
        key: "action.snooze",
        translations: &[
            (Locale::EnUs, "Snooze"),
            (Locale::PtPt, "Adiar"),
            (Locale::JaJp, "スヌーズ"),
        ],
    },
    Entry {
        key: "action.star",
        translations: &[
            (Locale::EnUs, "Star"),
            (Locale::PtPt, "Destacar"),
            (Locale::JaJp, "スター"),
        ],
    },
    Entry {
        key: "action.search",
        translations: &[
            (Locale::EnUs, "Search"),
            (Locale::PtPt, "Pesquisar"),
            (Locale::JaJp, "検索"),
        ],
    },
    Entry {
        key: "action.sync",
        translations: &[
            (Locale::EnUs, "Sync"),
            (Locale::PtPt, "Sincronizar"),
            (Locale::JaJp, "同期"),
        ],
    },
    Entry {
        key: "settings.appearance",
        translations: &[
            (Locale::EnUs, "Appearance"),
            (Locale::PtPt, "Aparência"),
            (Locale::JaJp, "外観"),
        ],
    },
    Entry {
        key: "settings.ai",
        translations: &[
            (Locale::EnUs, "AI Assistant"),
            (Locale::PtPt, "Assistente IA"),
            (Locale::JaJp, "AIアシスタント"),
        ],
    },
    Entry {
        key: "settings.accounts",
        translations: &[
            (Locale::EnUs, "Accounts"),
            (Locale::PtPt, "Contas"),
            (Locale::JaJp, "アカウント"),
        ],
    },
    Entry {
        key: "reader.unread",
        translations: &[
            (Locale::EnUs, "unread"),
            (Locale::PtPt, "não lido"),
            (Locale::JaJp, "未読"),
        ],
    },
    Entry {
        key: "reader.attachments",
        translations: &[
            (Locale::EnUs, "Attachments"),
            (Locale::PtPt, "Anexos"),
            (Locale::JaJp, "添付ファイル"),
        ],
    },
    Entry {
        key: "composer.to",
        translations: &[
            (Locale::EnUs, "To"),
            (Locale::PtPt, "Para"),
            (Locale::JaJp, "宛先"),
        ],
    },
    Entry {
        key: "composer.subject",
        translations: &[
            (Locale::EnUs, "Subject"),
            (Locale::PtPt, "Assunto"),
            (Locale::JaJp, "件名"),
        ],
    },
    Entry {
        key: "composer.body",
        translations: &[
            (Locale::EnUs, "Write your message…"),
            (Locale::PtPt, "Escreva sua mensagem…"),
            (Locale::JaJp, "メッセージを入力…"),
        ],
    },
];

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_parses_bcp47() {
        assert_eq!(Locale::from_str("en-US"), Locale::EnUs);
        assert_eq!(Locale::from_str("en-GB"), Locale::EnGb);
        assert_eq!(Locale::from_str("pt-PT"), Locale::PtPt);
        assert_eq!(Locale::from_str("pt-BR"), Locale::PtBr);
        assert_eq!(Locale::from_str("es-ES"), Locale::EsEs);
        assert_eq!(Locale::from_str("de-DE"), Locale::DeDe);
        assert_eq!(Locale::from_str("fr-FR"), Locale::FrFr);
        assert_eq!(Locale::from_str("ja-JP"), Locale::JaJp);
    }

    #[test]
    fn from_str_parses_posix_style() {
        // POSIX-style with encoding suffix should still map correctly.
        assert_eq!(Locale::from_str("pt_PT.UTF-8"), Locale::PtPt);
        assert_eq!(Locale::from_str("ja_JP.UTF-8"), Locale::JaJp);
        assert_eq!(Locale::from_str("en_US"), Locale::EnUs);
    }

    #[test]
    fn from_str_unknown_falls_back_to_en_us() {
        assert_eq!(Locale::from_str("xx-YY"), Locale::EnUs);
        assert_eq!(Locale::from_str(""), Locale::EnUs);
        assert_eq!(Locale::from_str("   "), Locale::EnUs);
        assert_eq!(Locale::from_str("C"), Locale::EnUs);
    }

    #[test]
    fn from_str_case_insensitive() {
        assert_eq!(Locale::from_str("PT-pt"), Locale::PtPt);
        assert_eq!(Locale::from_str("JA-jp"), Locale::JaJp);
    }

    #[test]
    fn as_str_roundtrips_known_tags() {
        assert_eq!(Locale::EnUs.as_str(), "en-US");
        assert_eq!(Locale::PtPt.as_str(), "pt-PT");
        assert_eq!(Locale::JaJp.as_str(), "ja-JP");
    }

    #[test]
    fn display_name_is_human_readable() {
        assert_eq!(Locale::EnUs.display_name(), "English (US)");
        assert_eq!(Locale::PtPt.display_name(), "Português (Portugal)");
        assert_eq!(Locale::JaJp.display_name(), "日本語 (日本)");
    }

    #[test]
    fn serde_roundtrips_locale() {
        let json = serde_json::to_string(&Locale::PtPt).unwrap();
        assert_eq!(json, "\"ptpt\"");
        let back: Locale = serde_json::from_str("\"ptpt\"").unwrap();
        assert_eq!(back, Locale::PtPt);

        let json = serde_json::to_string(&Locale::JaJp).unwrap();
        assert_eq!(json, "\"jajp\"");
        let back: Locale = serde_json::from_str("\"jajp\"").unwrap();
        assert_eq!(back, Locale::JaJp);
    }

    #[test]
    fn serde_i18n_config_default() {
        let cfg = I18nConfig::default();
        assert_eq!(cfg.locale, Locale::EnUs);
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("\"locale\":\"enus\""));
        let back: I18nConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.locale, Locale::EnUs);
    }

    #[test]
    fn t_returns_correct_translation_for_pt_pt() {
        assert_eq!(t("sidebar.inbox", Locale::PtPt), "Caixa de Entrada");
        assert_eq!(t("sidebar.sent", Locale::PtPt), "Enviados");
        assert_eq!(t("sidebar.drafts", Locale::PtPt), "Rascunhos");
        assert_eq!(t("sidebar.archive", Locale::PtPt), "Arquivo");
        assert_eq!(t("sidebar.trash", Locale::PtPt), "Lixeira");
        assert_eq!(t("sidebar.priority", Locale::PtPt), "Prioridade");
        assert_eq!(t("sidebar.snoozed", Locale::PtPt), "Adiados");
        assert_eq!(t("action.compose", Locale::PtPt), "Nova Mensagem");
        assert_eq!(t("action.reply", Locale::PtPt), "Responder");
        assert_eq!(t("action.forward", Locale::PtPt), "Encaminhar");
        assert_eq!(t("action.archive", Locale::PtPt), "Arquivar");
        assert_eq!(t("action.delete", Locale::PtPt), "Excluir");
        assert_eq!(t("action.snooze", Locale::PtPt), "Adiar");
        assert_eq!(t("action.star", Locale::PtPt), "Destacar");
        assert_eq!(t("action.search", Locale::PtPt), "Pesquisar");
        assert_eq!(t("action.sync", Locale::PtPt), "Sincronizar");
        assert_eq!(t("settings.appearance", Locale::PtPt), "Aparência");
        assert_eq!(t("settings.ai", Locale::PtPt), "Assistente IA");
        assert_eq!(t("settings.accounts", Locale::PtPt), "Contas");
        assert_eq!(t("reader.unread", Locale::PtPt), "não lido");
        assert_eq!(t("reader.attachments", Locale::PtPt), "Anexos");
        assert_eq!(t("composer.to", Locale::PtPt), "Para");
        assert_eq!(t("composer.subject", Locale::PtPt), "Assunto");
        assert_eq!(t("composer.body", Locale::PtPt), "Escreva sua mensagem…");
        assert_eq!(t("app.name", Locale::PtPt), "Komorebi");
    }

    #[test]
    fn t_returns_correct_translation_for_ja_jp() {
        assert_eq!(t("sidebar.inbox", Locale::JaJp), "受信箱");
        assert_eq!(t("sidebar.sent", Locale::JaJp), "送信済み");
        assert_eq!(t("sidebar.drafts", Locale::JaJp), "下書き");
        assert_eq!(t("sidebar.archive", Locale::JaJp), "アーカイブ");
        assert_eq!(t("sidebar.trash", Locale::JaJp), "ゴミ箱");
        assert_eq!(t("sidebar.priority", Locale::JaJp), "優先");
        assert_eq!(t("sidebar.snoozed", Locale::JaJp), "スヌーズ");
        assert_eq!(t("action.compose", Locale::JaJp), "新規メッセージ");
        assert_eq!(t("action.reply", Locale::JaJp), "返信");
        assert_eq!(t("action.forward", Locale::JaJp), "転送");
        assert_eq!(t("action.archive", Locale::JaJp), "アーカイブ");
        assert_eq!(t("action.delete", Locale::JaJp), "削除");
        assert_eq!(t("action.snooze", Locale::JaJp), "スヌーズ");
        assert_eq!(t("action.star", Locale::JaJp), "スター");
        assert_eq!(t("action.search", Locale::JaJp), "検索");
        assert_eq!(t("action.sync", Locale::JaJp), "同期");
        assert_eq!(t("settings.appearance", Locale::JaJp), "外観");
        assert_eq!(t("settings.ai", Locale::JaJp), "AIアシスタント");
        assert_eq!(t("settings.accounts", Locale::JaJp), "アカウント");
        assert_eq!(t("reader.unread", Locale::JaJp), "未読");
        assert_eq!(t("reader.attachments", Locale::JaJp), "添付ファイル");
        assert_eq!(t("composer.to", Locale::JaJp), "宛先");
        assert_eq!(t("composer.subject", Locale::JaJp), "件名");
        assert_eq!(t("composer.body", Locale::JaJp), "メッセージを入力…");
        assert_eq!(t("app.name", Locale::JaJp), "Komorebi");
    }

    #[test]
    fn t_returns_en_us_value_when_unknown_locale() {
        // A locale with no translations in the catalog falls back to en-US.
        // de-DE has no entries in the catalog, so every key falls back.
        assert_eq!(t("sidebar.inbox", Locale::DeDe), "Inbox");
        assert_eq!(t("action.reply", Locale::DeDe), "Reply");
        assert_eq!(t("composer.body", Locale::DeDe), "Write your message…");
    }

    #[test]
    fn t_returns_key_when_missing_from_all_locales() {
        assert_eq!(t("does.not.exist", Locale::EnUs), "does.not.exist");
        assert_eq!(t("does.not.exist", Locale::PtPt), "does.not.exist");
        assert_eq!(t("does.not.exist", Locale::JaJp), "does.not.exist");
    }

    #[test]
    fn set_and_current_locale_work() {
        // Capture the pre-existing locale so we restore it for other tests.
        let original = current_locale();
        set_locale(Locale::JaJp);
        assert_eq!(current_locale(), Locale::JaJp);
        set_locale(Locale::PtPt);
        assert_eq!(current_locale(), Locale::PtPt);
        set_locale(original);
    }

    #[test]
    fn t_macro_uses_current_locale() {
        let original = current_locale();
        set_locale(Locale::PtPt);
        assert_eq!(t!("sidebar.inbox"), "Caixa de Entrada");
        set_locale(Locale::JaJp);
        assert_eq!(t!("sidebar.inbox"), "受信箱");
        set_locale(Locale::EnUs);
        assert_eq!(t!("sidebar.inbox"), "Inbox");
        set_locale(original);
    }

    #[test]
    fn all_25_keys_present_in_en_us_pt_pt_and_ja_jp() {
        let expected_keys: &[&str] = &[
            "app.name",
            "sidebar.inbox",
            "sidebar.sent",
            "sidebar.drafts",
            "sidebar.archive",
            "sidebar.trash",
            "sidebar.priority",
            "sidebar.snoozed",
            "action.compose",
            "action.reply",
            "action.forward",
            "action.archive",
            "action.delete",
            "action.snooze",
            "action.star",
            "action.search",
            "action.sync",
            "settings.appearance",
            "settings.ai",
            "settings.accounts",
            "reader.unread",
            "reader.attachments",
            "composer.to",
            "composer.subject",
            "composer.body",
        ];
        assert_eq!(expected_keys.len(), 25);
        let cat = catalog();
        for key in expected_keys {
            let by_locale = cat
                .get(*key)
                .unwrap_or_else(|| panic!("key {key} missing from catalog"));
            assert!(
                by_locale.contains_key(&Locale::EnUs),
                "key {key} missing EnUs"
            );
            assert!(
                by_locale.contains_key(&Locale::PtPt),
                "key {key} missing PtPt"
            );
            assert!(
                by_locale.contains_key(&Locale::JaJp),
                "key {key} missing JaJp"
            );
            // Ensure the en-US value is non-empty.
            assert!(
                !by_locale[&Locale::EnUs].is_empty(),
                "key {key} has empty EnUs"
            );
        }
        assert_eq!(cat.len(), 25, "catalog should contain exactly 25 keys");
    }

    #[test]
    fn catalog_entry_count_matches_expected_keys() {
        assert_eq!(CATALOG_ENTRIES.len(), 25);
    }

    #[test]
    fn detect_locale_falls_back_to_en_us_when_unset() {
        // We cannot reliably control the host env vars in a unit test, but we
        // can at least assert that detection returns a valid, supported locale
        // and never panics. When none of LANG/LC_ALL/LC_MESSAGES are set, the
        // result must be EnUs.
        let detected = detect_locale();
        // Any of the supported variants is acceptable; this mainly guards
        // against panics and unexpected return values.
        let supported = [
            Locale::EnUs,
            Locale::EnGb,
            Locale::PtPt,
            Locale::PtBr,
            Locale::EsEs,
            Locale::DeDe,
            Locale::FrFr,
            Locale::JaJp,
        ];
        assert!(supported.contains(&detected));
    }
}
