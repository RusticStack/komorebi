# Translations


The text lives in the `.slint` files, each string in `@tr("...")` (whole
sentences with `{}` placeholders, `"one" | "many" % n` for counts, a context
`@tr("ctx" => "...")` where one English word needs two translations). Rust
reads its sentences back through the `Tr` global in `ui/tr.slint`
(`src/i18n.rs`, `src/tr.rs`), so there is one catalog. Slint bundles
`translations/<tag>/LC_MESSAGES/komorebi.po` at build time (no gettext at
run time); English is the source text.

To add a language: add `translations/<tag>/LC_MESSAGES/komorebi.po` (copy
`translations/komorebi.pot`, set the `Plural-Forms` header), add the `Lang`
variant and the date, number and size tables in `src/format.rs`, and the
Settings choice. After changing any string, regenerate the template with
`KOMOREBI_UPDATE_POT=1 cargo test --test i18n_catalog template` and fill the
new entries; `cargo test --test i18n_catalog` fails on a missing or fuzzy
translation, a changed placeholder, a stale entry, Brazilian wording in
pt-PT, and any visible literal outside `@tr` that is not in
`translations/untranslated-allowlist.txt`. Check the wide layout and the
compact one (below 1040 px) and one pane (below 720 px, e.g.
`KOMOREBI_SIZE=560x700`) in each language with `KOMOREBI_LANG=pt-PT`
(or `en`) and `KOMOREBI_SNAPSHOT`. English dates with a year follow
Windows' short-date format (Settings › Time & language › Region), read
once at start; ISO 8601 when it can't be read.

