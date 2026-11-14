//! The display language in the GPUI app (docs/GPUI-I18N.md).
//!
//! The language lives in a GPUI global, [`CurrentLang`], which views read
//! and observe, and in `komorebi_i18n`'s process-wide value, which `tr!`
//! and `trn!` read (also off the UI thread and in the library). Only
//! [`init`] and [`set_language`] change them, always together. Switching
//! refreshes every window, so every `tr!` in a `render` runs again in the
//! new language (cached views included: a refresh re-renders them); text
//! held outside `render` (the OS window title, a Kit input's placeholder)
//! is updated by an observer of [`CurrentLang`].
//!
//! GPUI Kit translates its own built-in text (an input's Cut, Copy and
//! Paste, a dialog's Cancel, the calendar's names) from its own table; the
//! vendored copy has a `pt-PT` column (`third_party/gpui-component/
//! locales/ui.yml`, KOMOREBI-PATCH.md), and [`kit_locale`] is the locale it
//! is switched to together with the language.
//!
//! Under `cfg(test)` the value `tr!` reads is the test thread's own
//! (`komorebi_i18n::set_thread_lang`), so parallel tests never share it;
//! Kit's locale is process-wide, so tests leave it alone and call Kit's
//! lookup with a locale instead.

use gpui::{App, Global};
use komorebi::i18n::Lang;

/// The language every window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CurrentLang(pub Lang);

impl Global for CurrentLang {}

/// Sets the language at start-up (no window to refresh yet).
pub fn init(lang: Lang, cx: &mut App) {
    store(lang);
    cx.set_global(CurrentLang(lang));
}

/// The language shown (English before [`init`]).
// The Settings view (phase 6) is the first caller; until then the tests.
#[cfg_attr(not(test), allow(dead_code))]
pub fn lang(cx: &App) -> Lang {
    cx.try_global::<CurrentLang>().map_or(Lang::En, |g| g.0)
}

/// Shows the app in `lang`: sets the global and `tr!`'s value, notifies
/// the observers of [`CurrentLang`] and redraws every window. `false` when
/// it was already shown in `lang` (nothing happens).
// The Settings view's language choice (phase 6) calls it; until then the
// tests.
#[cfg_attr(not(test), allow(dead_code))]
pub fn set_language(lang: Lang, cx: &mut App) -> bool {
    if cx.try_global::<CurrentLang>().map(|g| g.0) == Some(lang) {
        return false;
    }
    init(lang, cx);
    cx.refresh_windows();
    true
}

/// The locale Kit's own strings are looked up in for `lang`: the tag of its
/// catalog (`en`, `pt-PT`), which is also the name of the column in Kit's
/// `ui.yml`.
pub fn kit_locale(lang: Lang) -> &'static str {
    lang.tag()
}

fn store(lang: Lang) {
    #[cfg(not(test))]
    {
        komorebi_i18n::set_current(lang);
        gpui_kit::component::set_locale(kit_locale(lang));
    }
    #[cfg(test)]
    komorebi_i18n::set_thread_lang(Some(lang));
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, IntoElement, Render, TestAppContext, Window, div, prelude::*};
    use komorebi_i18n::tr;

    /// A view that remembers what its last render said.
    struct Probe {
        said: String,
        renders: usize,
    }

    impl Render for Probe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.renders += 1;
            self.said = tr!("Minimize");
            div().child(self.said.clone())
        }
    }

    #[gpui::test]
    fn switching_the_language_redraws_in_it(cx: &mut TestAppContext) {
        cx.update(|cx| init(Lang::En, cx));
        let (view, cx) = cx.add_window_view(|_, _| Probe {
            said: String::new(),
            renders: 0,
        });
        cx.run_until_parked();
        let (said, before) = view.read_with(cx, |v, _| (v.said.clone(), v.renders));
        assert_eq!(said, "Minimize");
        assert!(before >= 1);

        assert!(cx.update(|_, cx| set_language(Lang::PtPt, cx)));
        cx.run_until_parked();
        let (said, after) = view.read_with(cx, |v, _| (v.said.clone(), v.renders));
        assert_eq!(said, "Minimizar");
        assert!(after > before, "the refresh re-rendered the view");
        assert_eq!(cx.update(|_, cx| lang(cx)), Lang::PtPt);

        // The same language again: no refresh.
        assert!(!cx.update(|_, cx| set_language(Lang::PtPt, cx)));
        assert!(cx.update(|_, cx| set_language(Lang::En, cx)));
        cx.run_until_parked();
        assert_eq!(view.read_with(cx, |v, _| v.said.clone()), "Minimize");
    }

    #[gpui::test]
    fn observers_hear_the_switch(cx: &mut TestAppContext) {
        use std::cell::RefCell;
        use std::rc::Rc;
        cx.update(|cx| init(Lang::En, cx));
        let heard: Rc<RefCell<Vec<Lang>>> = Rc::default();
        let sink = heard.clone();
        let _subscription = cx.update(|cx| {
            cx.observe_global::<CurrentLang>(move |cx| sink.borrow_mut().push(lang(cx)))
        });
        cx.update(|cx| set_language(Lang::PtPt, cx));
        cx.run_until_parked();
        assert_eq!(heard.borrow().last(), Some(&Lang::PtPt));
    }

    /// Kit's own text (`gpui_kit::component`'s `t!`) in the language: its
    /// lookup, asked for a locale (the process-wide one stays English in
    /// tests).
    fn kit(lang: Lang, key: &str) -> String {
        gpui_kit::component::_rust_i18n_translate(kit_locale(lang), key).into_owned()
    }

    #[test]
    fn kit_follows_the_language() {
        assert_eq!(kit_locale(Lang::En), "en");
        assert_eq!(kit_locale(Lang::PtPt), "pt-PT");
        assert_eq!(kit(Lang::En, "Input.Copy"), "Copy");
        assert_eq!(kit(Lang::PtPt, "Input.Copy"), "Copiar");
        assert_eq!(kit(Lang::PtPt, "Dialog.cancel"), "Cancelar");
        assert_eq!(
            kit(Lang::PtPt, "Carousel.slide"),
            "Diapositivo %{current} de %{total}"
        );
    }

    /// The vendored table has a Portuguese entry for every key it has an
    /// English one for (the owner's rule: Kit's built-in strings in pt-PT),
    /// with the same `%{..}` placeholders, and none of Kit's keys shows its
    /// English text in Portuguese by falling back.
    #[test]
    fn every_kit_string_is_in_portuguese() {
        let table = include_str!("../../../third_party/gpui-component/locales/ui.yml");
        let mut keys = 0;
        let (mut section, mut key) = (String::new(), String::new());
        for line in table.lines().map(str::trim_end) {
            if !line.starts_with(' ') && line.ends_with(':') && !line.starts_with('_') {
                section = line.trim_end_matches(':').to_owned();
            } else if let Some(k) = line
                .strip_prefix("  ")
                .filter(|k| !k.starts_with(' '))
                .and_then(|k| k.strip_suffix(':'))
            {
                key = format!("{section}.{k}");
                keys += 1;
                let (en, pt) = (kit(Lang::En, &key), kit(Lang::PtPt, &key));
                assert_ne!(
                    en, key,
                    "{key}: the English text is the key itself (not in the table)"
                );
                let holes = |s: &str| {
                    let mut h: Vec<String> = s
                        .split("%{")
                        .skip(1)
                        .filter_map(|r| r.split('}').next().map(str::to_owned))
                        .collect();
                    h.sort();
                    h
                };
                assert_eq!(holes(&en), holes(&pt), "{key}: the placeholders differ");
                assert!(
                    pt != en || matches!(en.as_str(), "OK" | "HSLA" | "Original"),
                    "{key}: no Portuguese text (shows {en:?})"
                );
            }
        }
        assert!(
            keys >= 119,
            "the table has {keys} keys, expected at least 119"
        );
        assert!(!key.is_empty());
    }
}
