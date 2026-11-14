# Text and languages in the app

How the app (Komorebi 1.0, `crates/komorebi-gpui`) shows text in English and European
Portuguese (pt-PT, AO90, never Brazilian). For the phase agents: read this before writing any
view. The plan: [GPUI-MIGRATION.md](GPUI-MIGRATION.md) phase 15; the catalog rules:
[TRANSLATIONS.md](TRANSLATIONS.md) and CONTRIBUTING.md.

## One catalog

The app and the library read one gettext catalog (the 0.7 Slint app read it too, until it was deleted at the cut-over):

| Path | What |
|---|---|
| `translations/komorebi.pot` | The template: every message of the Rust sources (`tr!`, `trn!`), generated. |
| `translations/pt-PT/LC_MESSAGES/komorebi.po` | The European Portuguese catalog (msgctxt, plurals, glossary). `komorebi-i18n` builds it into a table. |
| `translations/untranslated-allowlist.txt` | Literals that are shown but not translated (product, provider and font names, example hosts, language names). Empty now. |
| `crates/komorebi-i18n` | `Lang`, the current language, `tr!`, `trn!`, the lookup. No dependencies. |
| `crates/komorebi-i18n/po.rs` | The `.po` reader, shared by the crate's build script and `tests/i18n_catalog.rs`. |
| `crates/komorebi-gpui/src/i18n.rs` | The GPUI global `CurrentLang`, `init`, `set_language`. |
| `crates/komorebi-gpui/src/text.rs` | Sentences made from typed values (`say`, the window title, the tray, toasts). |
| `tests/i18n_catalog.rs` | The extractor, the template check and every catalog rule. |

A message is (context, text, plural). A `tr!` with the same text and context as an existing
entry is the same entry: it is already translated, and the template only gains the Rust file in
its `#:` line. A changed comma is a new, untranslated entry.

## The macros

```rust
use komorebi_i18n::{tr, trn};

tr!("Settings")                                   // msgid "Settings"
tr!("Edit {0}", account.name)                     // arguments: anything Display
tr!("status" => "Sent")                           // msgctxt "status" (not the Sent folder)
tr!("{0} didn't answer on port {1}. ...", host, port)
trn!("{n} new message", "{n} new messages", count)              // plural, no context
trn!("ctx" => "{n} remote image from {0} blocked.", "{n} remote images from {0} blocked.", n, sender)
```

- The text and the context must be **string literals**: the extractor reads them from the
  source. A `tr!` whose first argument is not a literal fails `cargo test --test i18n_catalog`.
- Placeholders: `{}` takes the next argument, `{0}` and `{1}` a given one (do not mix
  the two styles in one message), `{n}` is the count of `trn!` (required in both forms), `{{`
  and `}}` are braces. The catalog test checks that every call passes exactly as many arguments
  as its text has placeholders, and that a translation keeps the same placeholders.
- The count of `trn!` is any integer type; the plural rule is the catalog header's
  (`plural=(n != 1)` for pt-PT). `{n}` prints the plain number; a formatted
  count (`thousands`) goes in as an argument instead.
- Both return a `String` in the current language. A message the catalog does not have, or has
  only fuzzy or empty, shows in English; it never panics. A placeholder without an argument stays
  visible as written.
- Whole sentences only, never glued pieces: word order and plurals differ in Portuguese. No
  leading or trailing space (layout is the view's business). No en or em dashes.
- Machine text stays English and outside `tr!`: logs (`eprintln!`), panics and `expect`, error
  reasons under a dialog's text, protocol strings, settings keys, ids.

## The current language

- `komorebi_i18n::current()` is what the macros read: a per-thread override (tests) or the
  process-wide value. The app sets it through `crate::i18n`.
- In the GPUI app the language lives in the global `crate::i18n::CurrentLang`. Start-up sets
  the Windows display language first (for the DirectX pre-flight dialog, before the cache is
  read), then `i18n::init(lang, cx)` with the saved choice (`Settings::language`, or
  `KOMOREBI_LANG`), resolved by `komorebi::i18n::resolve_here`.
- Switching (the Settings view, phase 6): `crate::i18n::set_language(lang, cx)`. It sets the
  global and the macros' value together, notifies observers of `CurrentLang`, and calls
  `cx.refresh_windows()`: every window re-renders, cached views included (a refresh bypasses the
  view cache and the splice), so every `tr!` in a `render` runs again. Saving the choice to the
  settings is the caller's job.
- Text held **outside** `render` does not follow by itself: the OS window title (already handled
  in `window.rs` with `cx.observe_global_in::<CurrentLang>` and `window.set_window_title`), a
  Kit `InputState` placeholder, a tray menu, a toast already shown. Observe `CurrentLang` and set
  them again.
- Never cache translated `String`s in view state. Keep the typed value and translate in
  `render`.

## Sentences from typed values

The library and the workers produce no sentences; they hand over typed values
(`mail::errors::Say`, `Notice`, `Outcome`, a validation failure, a count). The app renders them
on the UI thread:

1. The view keeps the typed value in its state (`Option<Say>`, not `Option<String>`).
2. `render` calls a function of `crate::text` (or of the view's own module) that matches the
   value to a sentence with `tr!` inside, for example `text::say(&say)`.
3. A language switch refreshes the window, so the sentence is rebuilt in the new language.

Examples, with English and pt-PT tests (`crates/komorebi-gpui/src/text.rs`): `window_title`,
`start_failed`, `tray_open`, `tray_tooltip` (plural), `toast_burst` (plural), `sent` (context),
and the whole `say(&Say)` mapping. Also the caption buttons' names (`window.rs`) and the
DirectX pre-flight message (`preflight.rs`).

To add one: write the function with `tr!` inside, run
`KOMOREBI_UPDATE_POT=1 cargo test --test i18n_catalog template`, check
`git diff translations/komorebi.pot` (a new `msgid` needs its translation) and test the English
text and the Portuguese against the `.po`, with `komorebi_i18n::with_lang`.

## Tests

- `komorebi_i18n::with_lang(Lang::PtPt, || ...)` runs code in a language on the current thread
  only; tests run in parallel and must not touch the process-wide value.
- `#[gpui::test]` runs on the test thread, and under `cfg(test)` `crate::i18n` stores the
  language as that thread's override, so `set_language` in one test never leaks into another.
  `crate::i18n::tests` shows a switch re-rendering a view and observers hearing it.
- The catalog: `cargo test --test i18n_catalog` (root package). The crate: `cargo test -p
  komorebi-i18n` (lookup, plurals, placeholders, fallback, sorted tables).

## Developer workflow

After adding or changing any text in the app:

```powershell
$env:KOMOREBI_UPDATE_POT = 1; cargo test --test i18n_catalog template; Remove-Item Env:KOMOREBI_UPDATE_POT
git diff translations/komorebi.pot          # the new or changed entries
# translate them in translations/pt-PT/LC_MESSAGES/komorebi.po
# (glossary: ficheiro, ecrã, definições, palavra-passe, utilizador, separador; AO90)
cargo test --test i18n_catalog              # complete, current, no fuzzy, placeholders, glossary, literals
cargo test -p komorebi-i18n -p komorebi-gpui
```

The `.po` change reaches the GPUI app at its next build (the build script reruns when a
catalog changes); no other step. Check a view in both languages with `KOMOREBI_LANG=pt-PT` (or
`en`) on a throwaway cache; Portuguese runs 15 to 30 percent longer, so wrap or flex rather than
clip.

## What the catalog test enforces

The template equals what the extractor produces (Rust sources, in source order); every
message is whole (no outer space) and a plural carries `{n}`; each `.po` has a UTF-8 header,
`Plural-Forms` and its `Language`; no duplicate, stale (in no `tr!`), fuzzy or
placeholder-mismatched entry; every message translated; no Brazilian wording in pt-PT; the
argument count of every `tr!`/`trn!`; and no shown literal in the app outside `tr!`.

The shown-literal check is a heuristic over `crates/komorebi-gpui/src` (test modules,
attributes, comments and `tr!` arguments skipped; literals inside `eprintln!`, `println!`,
`write!`, `panic!`, the `assert!` family, `unreachable!`, `anyhow!`, `bail!`, `env!`,
`include_str!` and the like, and in `.expect(...)`, `.context(...)` and `.with_context(...)`,
never count). A literal is
flagged when it is given straight to an element's text or name (`.child`, `.aria_label`,
`.label`, `.tooltip`, `.placeholder`, `.title`, `.set_window_title`, `.description`,
`.message`, `.text`, `.header`, `.caption`, `.summary`, `.detail`, `title: Some(...)`,
`SharedString::from`) and has two letters in a row, or anywhere when it reads like a sentence (a
capital, three words, a full stop). Names that are not translated go in the allowlist (an
entry no source uses any more is stale). It cannot see text built
at run time from pieces: review for that.

The Rust extractor scans `src/` (the library) and
`crates/komorebi-gpui/src`. It counts arguments by top-level commas, so a turbofish with two
type parameters directly in a `tr!` argument (`f::<A, B>(x)`) would miscount: bind it to a
variable first.

## Format helpers (dates, numbers, sizes)

In the library, `src/format.rs`: the date, time, number and size helpers (`time`, `list_date`,
`weekday_full`, `weekday_short`, `reader_date`, `full_date`, `size`, `thousands`, `picture`,
`set_short_date`, `load_short_date`, `first_weekday`, and the name helpers `sender`, `recipients`,
`initials`, `file_kind`) take a `Lang` where they need one and use only chrono and Win32
(`GetLocaleInfoEx`). The calendar's day and month helpers stay for 1.1.0.

## Known gaps

- **GPUI Kit's own strings** (done in stage D). Kit translates its built-in text (an input's
  Clear, Cut, Copy and Paste, a dialog's OK and Cancel, the calendar's weekday and month names,
  select and combo box placeholders, 119 keys) with `rust-i18n` from
  `third_party/gpui-component/locales/ui.yml`. The vendored copy now has a `pt-PT` line on every
  key (a KOMOREBI PATCH, `third_party/gpui-component/KOMOREBI-PATCH.md`), and
  `crate::i18n::init` / `set_language` switch Kit's locale (`gpui_kit::component::set_locale`) with the
  language, so `cx.refresh_windows()` redraws Kit's text too. Checks: `i18n::tests::
  every_kit_string_is_in_portuguese` (every key has Portuguese text with the same `%{..}`
  placeholders; it also fails when a stale incremental build kept the old table) and
  `tests/i18n_catalog.rs` `kit_strings_agree_with_the_catalog` (the words Kit shares with the
  app's catalog, such as Copiar and Cancelar, are the catalog's; no Brazilian wording). A new Kit
  key needs its `pt-PT` line when Kit moves to a newer release. Kit's text that Komorebi already
  passes in from its own catalog (labels, placeholders) is unchanged.
- `{n}` and the numbers inside sentences are plain digits.
- Done at the cut-over: the Slint extractor, `ui/`, `ui/tr.slint`, `src/tr.rs` and the Slint
  call in `komorebi::i18n::select` (the function) are gone; the template comes from the Rust
  sources only, and the 191 pt-PT entries only the Slint app used were removed from the `.po`
  (1,094 entries now); the table's size counts in the optimization pass.
