# Komorebi patch: gpui-component 0.8.0

On top of the Skua copy (`SKUA-PATCH.md`). Every Komorebi line is marked `KOMOREBI PATCH`.

## European Portuguese for Kit's built-in text (`locales/ui.yml`)

**Problem.** Kit translates its own text (an input's Clear, Cut, Copy, Paste and Select All, the
show and hide password buttons, a dialog's OK and Cancel, the select, combo box, command and
list placeholders, the calendar's weekday and month names, the carousel, table, chart, questionnaire,
dictation and diff labels: 119 keys) with `rust-i18n` from `locales/ui.yml`, which has English,
Chinese and a few Italian weekdays, no Portuguese. In Komorebi's pt-PT those strings showed in
English (docs/GPUI-I18N.md "Known gaps").

**Change.** A `pt-PT` line on every key of `ui.yml` (the file's first lines say so). No Rust
change for the strings themselves (one line in `src/lib.rs`, below, keeps the
table current). The words that exist in Komorebi's own catalog
(`translations/pt-PT/LC_MESSAGES/komorebi.po`) are the catalog's: Remover, Tentar novamente,
Cancelar, Copiar, Cortar, Colar, Limpar, Selecionar tudo, Mostrar e Ocultar palavra-passe, Fechar,
Desfazer, Ignorar, Mais opções. The calendar's weekdays are the library's short forms (`dom.`,
`seg.`), the months lower case as the library writes them (`outubro`).

**What the app does.** `crates/komorebi-gpui/src/i18n.rs` calls
`gpui_kit::component::set_locale("pt-PT")` (or `"en"`) wherever it sets the language, and
`cx.refresh_windows()` redraws what Kit shows. Its test `every_kit_string_is_in_portuguese` reads
this file and fails on a key without a `pt-PT` line, or whose `%{..}` placeholders differ from the
English text; `tests/i18n_catalog.rs` `kit_strings_agree_with_the_catalog` keeps the shared words
equal to the catalog.

**Moving to a newer Kit.** Take the new `ui.yml`, add the `pt-PT` line to every new key (the test
names them), and drop this patch when Kit ships Portuguese.

## `build.rs` watches `locales`

Kit's build script printed `rerun-if-changed` for `build.rs` and the icons only, which makes cargo
look at those paths alone: an edit of `locales/ui.yml` did not rebuild the crate, so the table kept
the old strings. One more line, `cargo:rerun-if-changed=locales`.

## `i18n!` is expanded again (`src/lib.rs`)

`rust_i18n::i18n!("locales", fallback = "en")` reads `ui.yml` while it expands, but rustc's
incremental cache keys a proc macro's output on its input tokens, not on the files it reads. A
`target` folder that had built Kit before the `pt-PT` column existed therefore kept the old table
(the build said "Compiling gpui-component" and the Portuguese strings were still missing). The call
now reads `fallback = ["en"]` (the same fallback, other tokens), so such a folder expands it once
more. Editing `ui.yml` later has the same effect on an incremental build: delete
`target/debug/incremental/gpui_component-*`. A CI build (`CARGO_INCREMENTAL=0`) and a fresh folder
are not affected. The test `i18n::tests::every_kit_string_is_in_portuguese` fails when the table is
stale.

## A dialog host named by its title, and an alert host (`src/dialog/dialog.rs`)

**Gap.** Kit's `Dialog` gave its host node (the Dialog node that takes the focus) no accessible
name: `title()` takes an element, so Narrator said "dialog" and then the first button. A
confirmation of a destructive action (Delete permanently, Empty folder) was a plain Dialog where
Windows expects an AlertDialog (docs/GPUI-MIGRATION.md "Gaps: shell").

**Change.** `Dialog::a11y_label(text)` names the host (through `gpui_base::Dialog::label`, the
gpui-base patch in `third_party/gpui-base/KOMOREBI-PATCH.md`), and `Dialog::alert(cx)` builds the
host on `gpui_base::AlertDialog` (Kit's own `with_base_alert_dialog`) while keeping the dialog's
own content and layout. A private `BaseDialogRoot::label_opt` passes the name to either root.
Every changed line is marked `KOMOREBI PATCH`.

**What the app does.** `crates/komorebi-gpui/src/widgets/dialog.rs` `confirm_or` names every
confirm by its title and makes a destructive one an AlertDialog; the test
`confirm_runs_once_and_closes_cancel_only_closes` finds the title on one AlertDialog node and no
plain Dialog.

**Moving to a newer Kit.** Drop this when Kit names the host from `title()` or takes a label.

## `Input::aria_expanded` (`src/input/input.rs`)

**Gap.** An address field with a list of suggestions under it (the composer's To, Cc and Bcc)
could not tell assistive technology that the list was open: Kit's `Input` had no setter for the
expanded state (docs/GPUI-MIGRATION.md "Gaps: settings").

**Change.** `Input::aria_expanded(bool)` stores an `Option<bool>` and sets the node's expanded
state when it is set. Every changed line is marked `KOMOREBI PATCH`.

**What the app does.** `widgets::field::ChipField::expanded` passes it; the composer sets it while
the suggestions of that field show (`views/composer/recipients.rs`).

**Moving to a newer Kit.** Drop this when Kit's `Input` reports the expanded state itself.

## Access keys on menu items (`src/menu/popup_menu.rs`, `src/menu/menu_item.rs`)

**Gap.** A popup menu item could not have an access key (a mnemonic): nothing told assistive
technology a letter for it, nothing drew one, and a letter typed in an open menu did nothing. The
app's menus (the reader's More menu and the message row menu) had no chip and no screen reader
letter for Mark as read, Mark as unread, Save as .eml and the other commands with a key
(docs/GPUI-MIGRATION.md "Gaps integration").

**Change.** General, for upstream (Skua's fork):

- `PopupMenuItem::access_key(char)` on `Item`, `ElementItem` and `Submenu` (a new
  `access_key: Option<char>` field in each of those variants; the constructors set `None`, and the
  one exhaustive `Submenu` pattern in `render_item` takes `..`).
- `MenuItemElement::access_key(key, shown)`: the item node's `aria_keyshortcuts` is the upper-case
  letter (AccessKit's `keyboard_shortcut`, what UI Automation reads), and while `shown` a small
  chip with the letter (theme `foreground` on `background`, id `("menu-access-key", letter)`) ends
  the row. Disabled items get neither.
- `PopupMenu`: a letter or digit pressed in the open menu (no Ctrl, Windows key or Fn) runs the
  first enabled item whose access key it is, as Enter on it does, or selects and enters a submenu
  (`on_access_key`); the chips show while Alt is held on its own (`on_access_modifiers`, a new
  `access_keys_shown` field). A menu with no access keys behaves as before.

Every changed line is marked `KOMOREBI PATCH`.

**What the app does.** `crates/komorebi-gpui/src/views/mail/menus.rs` `fill` gives each entry whose
command has an access key (`access_of`) that key's letter in the current language; the keyboard's
own row menu (Shift+F10, drawn by the app) does the same. Tests:
`window::access_tests::menu_items_take_their_access_keys` and
`the_keyboards_row_menu_takes_the_letters`.

**Moving to a newer Kit.** Drop this when Kit's menu items take an access key.

## `native-menu-svg`: the second SVG stack is optional (`Cargo.toml`, `src/native_menu/windows.rs`)

**Problem.** On Windows, `NativeMenu` rasterizes an SVG icon of a menu item (`menu_with_icon`) with
`resvg` 0.45.1 (`rasterize_svg`, its only user). GPUI links `resvg` 0.46 for its own SVG icons, so the
exe held two SVG stacks: `resvg`, `usvg`, `svgtypes` and `kurbo` twice, and with the older one a second
`zune-jpeg`, `zune-core`, `gif` and `imagesize`.

**Change.** `resvg` is an optional dependency behind a new feature, `native-menu-svg`, off by default
in this copy. Without it `rasterize_svg` returns `None`, so an SVG icon is left out of its menu item.
Nothing else changes: the native menu, its text, its checks and its PNG, JPEG and BMP icons (GDI+)
are the same. Komorebi never gives a native menu an icon (the only native menu is an input's
Cut, Copy, Paste and Select All), so nothing it shows changes. The default stays off here because
`gpui-kit` depends on this crate with its default features and a downstream crate cannot switch a
default off; Skua's copy of the patch (`native-menu-svg` in `default`) keeps the icons.

**Measured (release, one worktree, the same profile).** Exe 18,817,024 bytes before, 18,388,992 after:
428,032 bytes smaller (2.27 percent). `Cargo.lock` loses `resvg` 0.45.1, `usvg` 0.45.1, `svgtypes` 0.15.3,
`kurbo` 0.11.3, `zune-jpeg` 0.4.21, `zune-core` 0.4.12, `gif` 0.13.3 and `imagesize` 0.13.0. (`tiny-skia`
0.11 and `png` 0.17 stay: GPUI's own `resvg` 0.46 uses them.)

**Moving to a newer Kit.** Drop this when Kit uses the same `resvg` as GPUI.
