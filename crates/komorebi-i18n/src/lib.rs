//! Komorebi's text for Rust code: the gettext catalog
//! (`translations/komorebi.pot`, `translations/<tag>/LC_MESSAGES/
//! komorebi.po`), read without gettext. The build script turns each
//! `.po` file into a static table keyed by (msgctxt, msgid); the macros look
//! a message up in the current language and fill its placeholders.
//!
//! ```
//! use komorebi_i18n::{Lang, tr, trn, with_lang};
//! with_lang(Lang::PtPt, || {
//!     assert_eq!(tr!("Settings"), "Definições");
//!     assert_eq!(tr!("tray menu" => "Quit"), "Sair");
//!     assert_eq!(trn!("{n} new message", "{n} new messages", 3), "3 mensagens novas");
//! });
//! ```
//!
//! Placeholders: `{}` takes the next argument, `{0}`, `{1}` a given one,
//! `{n}` the count of `trn!`, `{{` and `}}` are braces. A message the
//! catalog lacks (or has only fuzzy or empty) shows in English, the source
//! language.
//!
//! The current language is a process-wide value ([`set_current`]); a GPUI
//! app keeps it in step with its own global (`komorebi-gpui`'s `i18n`
//! module). Tests use [`with_lang`], which overrides it on one thread only.
//!
//! Every `tr!` and `trn!` in the scanned sources (`src/`, the GPUI app) is
//! extracted into the template by `KOMOREBI_UPDATE_POT=1 cargo test --test
//! i18n_catalog template`; their text must be string literals.

use std::cell::Cell;
use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};

/// A language the app is shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Lang {
    /// English, the source language (no catalog).
    #[default]
    En,
    /// European Portuguese (Portugal; never Brazilian).
    PtPt,
}

impl Lang {
    /// Every language, English first.
    pub const ALL: [Lang; 2] = [Lang::En, Lang::PtPt];

    /// The name of its catalog folder under `translations/` (`en` is the
    /// source text and has none).
    pub fn tag(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::PtPt => "pt-PT",
        }
    }

    fn index(self) -> u8 {
        match self {
            Lang::En => 0,
            Lang::PtPt => 1,
        }
    }

    fn from_index(i: u8) -> Lang {
        match i {
            1 => Lang::PtPt,
            _ => Lang::En,
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static THREAD: Cell<Option<Lang>> = const { Cell::new(None) };
}

/// The language `tr!` and `trn!` use on this thread: a [`with_lang`]
/// override, else the process-wide one.
pub fn current() -> Lang {
    THREAD
        .with(Cell::get)
        .unwrap_or_else(|| Lang::from_index(CURRENT.load(Ordering::Relaxed)))
}

/// Sets the process-wide language. In a GPUI app call the app's own switch
/// instead, which also sets its global and refreshes the windows.
pub fn set_current(lang: Lang) {
    CURRENT.store(lang.index(), Ordering::Relaxed);
}

/// Sets (or with `None` clears) this thread's override of the language.
/// For tests, where threads run in parallel and must not share it.
pub fn set_thread_lang(lang: Option<Lang>) {
    THREAD.with(|t| t.set(lang));
}

/// Runs `f` with `lang` as this thread's language, then restores the one
/// before (also when `f` panics).
pub fn with_lang<R>(lang: Lang, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Lang>);
    impl Drop for Restore {
        fn drop(&mut self) {
            set_thread_lang(self.0);
        }
    }
    let _restore = Restore(THREAD.with(|t| t.replace(Some(lang))));
    f()
}

/// How a language picks a plural form from a count (the catalog header's
/// `Plural-Forms`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plural {
    /// `plural=(n != 1)`: English, Portuguese.
    NotOne,
    /// `plural=(n > 1)`.
    #[allow(dead_code)]
    MoreThanOne,
    /// `nplurals=1; plural=0`.
    #[allow(dead_code)]
    One,
}

impl Plural {
    fn form(self, n: i64) -> usize {
        match self {
            Plural::NotOne => usize::from(n != 1),
            Plural::MoreThanOne => usize::from(n > 1),
            Plural::One => 0,
        }
    }
}

/// One translated message: context, source text, forms (one, or one per
/// plural form).
struct E {
    c: Option<&'static str>,
    i: &'static str,
    f: &'static [&'static str],
}

struct Catalog {
    tag: &'static str,
    plural: Plural,
    /// Sorted by (context, source text).
    entries: &'static [E],
}

mod generated {
    use super::{Catalog, E, Plural};
    include!(concat!(env!("OUT_DIR"), "/catalogs.rs"));
}

fn catalog(lang: Lang) -> Option<&'static Catalog> {
    if lang == Lang::En {
        return None;
    }
    generated::CATALOGS.iter().find(|c| c.tag == lang.tag())
}

fn entry(lang: Lang, ctx: Option<&str>, id: &str) -> Option<(&'static E, Plural)> {
    let catalog = catalog(lang)?;
    let at = catalog
        .entries
        .binary_search_by(|e| (e.c, e.i).cmp(&(ctx, id)))
        .ok()?;
    Some((&catalog.entries[at], catalog.plural))
}

/// The text of a message in `lang` (the source text when `lang` has no
/// translation of it).
pub fn translate(lang: Lang, ctx: Option<&'static str>, id: &'static str) -> &'static str {
    match entry(lang, ctx, id) {
        Some((e, _)) => e.f[0],
        None => id,
    }
}

/// The form of a counted message for `n` in `lang` (the English form when
/// `lang` has no translation of it).
pub fn translate_plural(
    lang: Lang,
    ctx: Option<&'static str>,
    one: &'static str,
    many: &'static str,
    n: i64,
) -> &'static str {
    if let Some((e, plural)) = entry(lang, ctx, one)
        && let Some(form) = e.f.get(plural.form(n))
    {
        return form;
    }
    if Plural::NotOne.form(n) == 0 {
        one
    } else {
        many
    }
}

/// Whether `lang`'s catalog translates the message (English: always).
pub fn is_translated(lang: Lang, ctx: Option<&str>, id: &str) -> bool {
    lang == Lang::En || entry(lang, ctx, id).is_some()
}

/// `template` with its placeholders filled: `{}` the next of `args`, `{0}`
/// a given one, `{n}` the count, `{{` and `}}` braces. A placeholder with
/// no argument stays as written, so a mistake shows instead of panicking.
pub fn fill(template: &str, n: Option<i64>, args: &[&dyn Display]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(template.len() + 16 * args.len());
    let mut next = 0;
    let mut rest = template;
    while let Some(at) = rest.find(['{', '}']) {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push_str(&tail[..1]);
            rest = &tail[2..];
            continue;
        }
        if let Some(after) = tail.strip_prefix('}') {
            out.push('}');
            rest = after;
            continue;
        }
        let Some(end) = tail.find('}') else {
            out.push_str(tail);
            rest = "";
            break;
        };
        let name = &tail[1..end];
        let value: Option<&dyn Display> = match name {
            "" => {
                next += 1;
                args.get(next - 1).copied()
            }
            "n" => n.as_ref().map(|n| n as &dyn Display),
            _ => name
                .parse::<usize>()
                .ok()
                .and_then(|i| args.get(i).copied()),
        };
        match value {
            Some(v) => {
                let _ = write!(out, "{v}");
            }
            None => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

#[doc(hidden)]
pub fn __tr(ctx: Option<&'static str>, id: &'static str, args: &[&dyn Display]) -> String {
    let text = translate(current(), ctx, id);
    if args.is_empty() && !text.contains(['{', '}']) {
        return text.to_string();
    }
    fill(text, None, args)
}

#[doc(hidden)]
pub fn __trn<N: TryInto<i64>>(
    ctx: Option<&'static str>,
    one: &'static str,
    many: &'static str,
    n: N,
    args: &[&dyn Display],
) -> String {
    let n = n.try_into().unwrap_or(i64::MAX);
    fill(
        translate_plural(current(), ctx, one, many, n),
        Some(n),
        args,
    )
}

/// A message in the current language, filled: `tr!("text {0}", arg)` or,
/// with a context (`msgctxt`), `tr!("context" => "text", args...)`. The
/// text must be a string literal (it is extracted into the template).
#[macro_export]
macro_rules! tr {
    ($ctx:literal => $id:literal $(, $arg:expr)* $(,)?) => {
        $crate::__tr(
            ::core::option::Option::Some($ctx),
            $id,
            &[$(&$arg as &dyn ::core::fmt::Display),*],
        )
    };
    ($id:literal $(, $arg:expr)* $(,)?) => {
        $crate::__tr(
            ::core::option::Option::None,
            $id,
            &[$(&$arg as &dyn ::core::fmt::Display),*],
        )
    };
}

/// A counted message in the current language, filled:
/// `trn!("{n} message", "{n} messages", count, args...)` or, with a
/// context, `trn!("context" => "{n} message", "{n} messages", count)`.
/// Both forms carry `{n}`; the count is any integer.
#[macro_export]
macro_rules! trn {
    ($ctx:literal => $one:literal, $many:literal, $n:expr $(, $arg:expr)* $(,)?) => {
        $crate::__trn(
            ::core::option::Option::Some($ctx),
            $one,
            $many,
            $n,
            &[$(&$arg as &dyn ::core::fmt::Display),*],
        )
    };
    ($one:literal, $many:literal, $n:expr $(, $arg:expr)* $(,)?) => {
        $crate::__trn(
            ::core::option::Option::None,
            $one,
            $many,
            $n,
            &[$(&$arg as &dyn ::core::fmt::Display),*],
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_but_english_has_a_catalog() {
        for lang in Lang::ALL {
            assert_eq!(catalog(lang).is_some(), lang != Lang::En, "{lang:?}");
        }
        let pt = catalog(Lang::PtPt).unwrap();
        assert_eq!(pt.plural, Plural::NotOne);
        assert!(pt.entries.len() > 1000, "{}", pt.entries.len());
    }

    #[test]
    fn tables_are_sorted_for_the_lookup() {
        for catalog in generated::CATALOGS {
            for pair in catalog.entries.windows(2) {
                assert!(
                    (pair[0].c, pair[0].i) < (pair[1].c, pair[1].i),
                    "{}: {:?} before {:?}",
                    catalog.tag,
                    pair[0].i,
                    pair[1].i
                );
            }
        }
    }

    #[test]
    fn messages_follow_the_language() {
        with_lang(Lang::En, || {
            assert_eq!(tr!("Settings"), "Settings");
            assert_eq!(tr!("Edit {0}", "Work"), "Edit Work");
        });
        with_lang(Lang::PtPt, || {
            assert_eq!(tr!("Settings"), "Definições");
            assert_eq!(tr!("Edit {0}", "Work"), "Editar Work");
            // A context picks the entry; the same text without one is another.
            assert_eq!(tr!("tray menu" => "Quit"), "Sair");
            assert_eq!(tr!("suggestion" => "{0}, {1}", "a", "b"), "a, b");
        });
    }

    #[test]
    fn counts_pick_the_plural_form() {
        with_lang(Lang::En, || {
            assert_eq!(
                trn!("{n} new message", "{n} new messages", 1),
                "1 new message"
            );
            assert_eq!(
                trn!("{n} new message", "{n} new messages", 0),
                "0 new messages"
            );
            assert_eq!(
                trn!("{n} new message", "{n} new messages", 2usize),
                "2 new messages"
            );
        });
        with_lang(Lang::PtPt, || {
            assert_eq!(
                trn!("{n} new message", "{n} new messages", 1),
                "1 mensagem nova"
            );
            assert_eq!(
                trn!("{n} new message", "{n} new messages", 0),
                "0 mensagens novas"
            );
            assert_eq!(
                trn!("{n} new message", "{n} new messages", 7u32),
                "7 mensagens novas"
            );
        });
    }

    #[test]
    fn a_missing_message_shows_in_english() {
        with_lang(Lang::PtPt, || {
            assert_eq!(tr!("Not in any catalog {0}", 5), "Not in any catalog 5");
            assert_eq!(tr!("no such context" => "Settings"), "Settings");
            assert_eq!(trn!("{n} zebra", "{n} zebras", 2), "2 zebras");
            assert_eq!(trn!("{n} zebra", "{n} zebras", 1), "1 zebra");
        });
        assert!(!is_translated(Lang::PtPt, None, "Not in any catalog {0}"));
        assert!(is_translated(Lang::PtPt, None, "Settings"));
        assert!(is_translated(Lang::En, None, "anything"));
    }

    #[test]
    fn placeholders_fill_by_position_and_number() {
        let a: &dyn Display = &"A";
        let b: &dyn Display = &2;
        assert_eq!(fill("{} and {}", None, &[a, b]), "A and 2");
        assert_eq!(fill("{1} before {0}", None, &[a, b]), "2 before A");
        assert_eq!(fill("{n} of {0}", Some(3), &[a]), "3 of A");
        assert_eq!(fill("{{literal}} {0}", None, &[a]), "{literal} A");
        assert_eq!(fill("a } b", None, &[]), "a } b");
        // Missing arguments stay visible, never panic.
        assert_eq!(fill("{0} {1} {n} {x}", None, &[a]), "A {1} {n} {x}");
        assert_eq!(fill("open {", None, &[]), "open {");
        assert_eq!(fill("", None, &[]), "");
        assert_eq!(fill("ção {0}", None, &[a]), "ção A");
    }

    #[test]
    fn the_thread_override_is_scoped() {
        // A thread override as the baseline, so the process-wide value
        // (another test sets it) cannot interfere.
        set_thread_lang(Some(Lang::En));
        let inside = with_lang(Lang::PtPt, || {
            let nested = with_lang(Lang::En, current);
            (current(), nested)
        });
        assert_eq!(inside, (Lang::PtPt, Lang::En));
        assert_eq!(THREAD.with(Cell::get), Some(Lang::En));
        let caught = std::panic::catch_unwind(|| with_lang(Lang::PtPt, || panic!("boom")));
        assert!(caught.is_err());
        assert_eq!(
            THREAD.with(Cell::get),
            Some(Lang::En),
            "restored after a panic"
        );
        set_thread_lang(None);
        assert_eq!(THREAD.with(Cell::get), None);
    }

    #[test]
    fn the_process_language_is_read_without_an_override() {
        // Only this test touches the process-wide value.
        set_thread_lang(None);
        set_current(Lang::PtPt);
        assert_eq!(current(), Lang::PtPt);
        assert_eq!(tr!("Settings"), "Definições");
        // Another thread sees it too.
        let other = std::thread::spawn(current).join().unwrap();
        assert_eq!(other, Lang::PtPt);
        set_current(Lang::En);
        assert_eq!(current(), Lang::En);
    }
}
