# Komorebi patches to gpui-base 0.8.0

The crates.io source of `gpui-base` 0.8.0 (the registry copy, without `.cargo-ok`), vendored for
three changes. Every changed line is marked `KOMOREBI PATCH`.

## The dialog host's accessible name (`src/dialog.rs`, `src/alert_dialog.rs`)

**Gap.** `Dialog` rendered its host node (role Dialog, or AlertDialog when built by
`AlertDialog`) with no accessible name, and nothing in the builder could set one, so Narrator
said "dialog" without the title.

**Change.** A `label: Option<SharedString>` field, `Dialog::label(text)`, and
`.when_some(self.label, |this, label| this.aria_label(label))` on the host; `AlertDialog::label`
passes it through. Nothing else changes; with no label the host renders as before.

**Used by.** gpui-component's `Dialog::a11y_label` (`third_party/gpui-component/KOMOREBI-PATCH.md`).

**Moving to a newer release.** Take the new registry source, re-apply the field, the two builder
methods and the one render line, or drop this folder (and its `[patch.crates-io]` line in the root
`Cargo.toml`) once gpui-base names the host itself.

## The keyboard shortcut in a test snapshot (`src/test_support.rs`)

**Gap.** A headless test could read an element's role, name, states and value from its
`ElementSnapshot`, but not the keyboard shortcut it tells assistive technology
(`aria_keyshortcuts`), so an access key's letter could not be asserted.

**Change.** A `keyshortcuts: Option<SharedString>` field filled from the node's
`keyboard_shortcut()` where the other facts are read, shown in `Debug`, and
`ElementSnapshot::keyshortcuts()`. Test support only; nothing changes at run time.

**Used by.** The access key tests of the app (`crates/komorebi-gpui/src/window/access_tests.rs`).

**Moving to a newer release.** Re-apply the field, the getter and the one line, or drop it once the
snapshot reads the shortcut itself.

## `text-formats`: the Markdown and HTML parsers are optional (`Cargo.toml`, `src/text/mod.rs`, `src/text/format_stub.rs`)

**Problem.** `TextView` and `TextViewState` parse Markdown (`markdown`) and HTML (`html5ever`,
`markup5ever_rcdom`) through `text::format`. Nothing in Komorebi renders either (no `TextView`,
`Text::markdown` or `Alert` with Markdown; the only other user is the hover popover of an input
with a language server, which Komorebi has none of), yet the parsers are reachable from
`TextViewState` and stayed in the exe through fat LTO.

**Change.** A `text-formats` feature, off by default in this copy, owns `html5ever` and
`markup5ever_rcdom` (now optional) and the real `text::format` module. Without it `text/mod.rs`
mounts `format_stub.rs` as `text::format`: `markdown::parse` and `html::parse` have the same
signatures and return an error, which a `TextView` shows as an empty document. The one production
call site (`state.rs`) and the `parse_markdown` helper of `editor_host` are unchanged. The
`markdown` crate stays a dependency because its `mdast` types are in the public plugin API; with
no caller of `to_mdast`, LTO drops its parser. `regex` and `lsp-types` stay: `imap` and
GPUI link `regex` anyway, and `lsp-types` is in the public input API. The default stays off here
because `gpui-kit` depends on this crate with its default features and a downstream crate
cannot switch a default off; Skua's copy of the patch has `text-formats` in `default`. Kit's own
tests that parse Markdown or HTML need `--features text-formats`.

**Measured (release, one worktree, the same profile).** Exe 18,388,992 bytes before (after the
`native-menu-svg` patch), 17,499,136 after: 889,856 bytes smaller (4.84 percent).
`Cargo.lock` loses `html5ever`, `markup5ever`, `markup5ever_rcdom`, `xml5ever`, `tendril`, `phf` 0.11 and its
generators, `string_cache` and `string_cache_codegen`, `futf`, `mac`, `utf-8` and `precomputed-hash`.

**Moving to a newer Kit.** Drop this when Kit makes its text formats optional.
