# Translations


The text is written in Rust with `tr!("ctx" => "...", args)` and
`trn!("one {n}", "many {n}", n)` from `crates/komorebi-i18n` (whole sentences
with `{}` placeholders, a context where one English word needs two
translations), whose build script turns `translations/<tag>/LC_MESSAGES/
komorebi.po` into a static table (no gettext at run time); English is the
source text. The app is `crates/komorebi-gpui` ([GPUI-I18N.md](GPUI-I18N.md)).

To add a language: add `translations/<tag>/LC_MESSAGES/komorebi.po` (copy
`translations/komorebi.pot`, set the `Plural-Forms` header), add the `Lang`
variant (`crates/komorebi-i18n/src/lib.rs`) and the date, number and size
tables in `src/format.rs`, and the Settings choice. After changing any
string, regenerate the template with
`KOMOREBI_UPDATE_POT=1 cargo test --test i18n_catalog template` (it reads
the `tr!`/`trn!` calls of `src/` and `crates/komorebi-gpui/src`) and fill the
new entries;
`cargo test --test i18n_catalog` fails on a missing or fuzzy translation, a
changed placeholder, a stale entry, Brazilian wording in pt-PT, a `tr!` with
the wrong number of arguments, and any visible literal outside `tr!`
that is not in `translations/untranslated-allowlist.txt`. Check the
wide layout and the compact one (below 1040 px) and one pane (below 720 px,
e.g. `KOMOREBI_SIZE=560x700`) in each language with `KOMOREBI_LANG=pt-PT`
(or `en`) and `KOMOREBI_SNAPSHOT`. English dates with a year follow
Windows' short-date format (Settings › Time & language › Region), read
once at start; ISO 8601 when it can't be read.

