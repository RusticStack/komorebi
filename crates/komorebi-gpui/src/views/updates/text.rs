//! The words of the updater and the What's new bar. Each `tr!` is the Slint
//! app's text and context (`ui/tr.slint`, `ui/app.slint`), translated already.

use komorebi_i18n::tr;

/// The bar after an update: "Komorebi was updated to 1.0.0."
pub fn whats_new(version: &str) -> String {
    tr!("Komorebi was updated to {0}.", version)
}

/// The update bar's sentence.
pub fn available(version: &str) -> String {
    tr!("Version {0} is available.", version)
}

pub fn download() -> String {
    tr!("Download and install")
}

/// Later (the context keeps it apart from other Laters).
pub fn later() -> String {
    tr!("update" => "Later")
}

pub fn release_notes() -> String {
    tr!("Release notes")
}

/// The bar's close.
pub fn hide_until_tomorrow() -> String {
    tr!("Hide the update bar until tomorrow")
}

#[cfg(test)]
mod tests {
    use super::*;
    use komorebi::i18n::Lang;
    use komorebi_i18n::with_lang;

    #[test]
    fn the_update_words_are_the_slint_apps() {
        with_lang(Lang::En, || {
            assert_eq!(available("1.2.0"), "Version 1.2.0 is available.");
            assert_eq!(later(), "Later");
        });
        with_lang(Lang::PtPt, || {
            for (pt, en) in [
                (download(), "Download and install"),
                (release_notes(), "Release notes"),
                (hide_until_tomorrow(), "Hide the update bar until tomorrow"),
            ] {
                assert_ne!(pt, en, "translated");
            }
            assert!(available("1.2.0").contains("1.2.0"));
        });
    }
}
