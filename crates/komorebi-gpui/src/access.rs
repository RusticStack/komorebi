//! Access keys (docs/GPUI-MIGRATION.md phase 13, WinUI 3 "Access keys"):
//! hold Alt and the commands show their letter in a small ink chip under
//! the control; Alt and the letter runs the command. The letters are
//! chosen per language (English "Reply" is Alt+R, Portuguese "Responder"
//! too; "Archive" and "Arquivar" are Alt+A, but "Folders" is Alt+O and
//! "Pastas" Alt+T), because a letter has to be in the name the person
//! reads.
//!
//! How it works, and why it is built from three small pieces:
//!
//! - **The key** is one [`Press`] action bound to Alt+A to Alt+Z
//!   ([`bind_keys`], installed once by the first control that has a key). Its
//!   handler turns the letter into a [`Key`] of the current language and
//!   sends the command's own action (the same one its shortcut sends) to
//!   the focused window, to the first that is available where the focus
//!   is. So an access key does exactly what the shortcut does and needs
//!   no code in the view; English Alt+S still sends in the composer (its
//!   own binding is deeper, so it wins).
//! - **The chip** appears while Alt is held: the window's root element
//!   calls [`AccessHost::access_keys`], which listens for the modifier
//!   change and redraws every window. A control that has an access key
//!   ([`Button::access_key`](crate::widgets::button::Button::access_key),
//!   [`IconButton::access_key`](crate::widgets::icon_button::IconButton::access_key))
//!   draws its chip then, and always tells UI Automation its letter
//!   (`aria-keyshortcuts`: "Alt+R"), which is what Narrator reads.
//! - **The menus** (More, the row menu, and the row menu the keyboard
//!   opens): an item whose command has a key takes its letter as the
//!   item's access key (the vendored Kit's `PopupMenuItem::access_key`,
//!   KOMOREBI-PATCH.md): the letter alone runs it while the menu is open,
//!   UI Automation is told the letter, and its chip shows at the item's end
//!   while Alt is held. Outside a menu the same keys (Mark as read and
//!   unread, Save as .eml) work as Alt and the letter wherever the message
//!   commands are available.
//! - **Settings** (its sections): each section has a key of its own
//!   ([`Group::Settings`]), drawn on the section list and sent as
//!   `settings::ShowSection`.
//! - **The table** ([`Key::letter`]) holds each key's letter per language.
//!   Keys that can be on screen together must differ ([`Group`]); the
//!   tests check that, and that the letter is in the command's name.

use gpui::{
    App, Context, Div, Global, InteractiveElement, IntoElement, KeyBinding, Modifiers,
    ParentElement, Role, Styled, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use komorebi::i18n::Lang;

use crate::theme::{ActiveTokens as _, TextStyled as _, Tokens, radius, space};
use crate::views::composer as compose;
use crate::views::mail::commands as mail;
use crate::views::settings::{self, Section};

/// Alt and a letter: the access key `0` of the current language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = komorebi, no_json)]
pub struct Press(pub char);

/// Which controls can be on screen together. Keys of the same group, and
/// of [`Group::Window`] with any group, must have different letters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// The window's own commands (title bar, folder pane, list header).
    Window,
    /// The open message's command bar.
    Message,
    /// The composer.
    Composer,
    /// Settings (its section list).
    Settings,
}

impl Group {
    /// Can a control of `self` and one of `other` be visible together?
    pub fn meets(self, other: Group) -> bool {
        self == Group::Window || other == Group::Window || self == other
    }
}

/// A command with an access key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    NewMail,
    Search,
    Refresh,
    Folders,
    Settings,
    Reply,
    ReplyAll,
    Forward,
    Archive,
    Delete,
    MoveTo,
    Important,
    MarkRead,
    MarkUnread,
    SaveEml,
    Send,
    Attach,
    Link,
    /// A section of Settings.
    Section(Section),
}

impl Key {
    /// Every key, in the order the table lists them.
    pub const ALL: [Key; 25] = [
        Key::NewMail,
        Key::Search,
        Key::Refresh,
        Key::Folders,
        Key::Settings,
        Key::Reply,
        Key::ReplyAll,
        Key::Forward,
        Key::Archive,
        Key::Delete,
        Key::MoveTo,
        Key::Important,
        Key::MarkRead,
        Key::MarkUnread,
        Key::SaveEml,
        Key::Send,
        Key::Attach,
        Key::Link,
        Key::Section(Section::Accounts),
        Key::Section(Section::Signatures),
        Key::Section(Section::Rules),
        Key::Section(Section::General),
        Key::Section(Section::Appearance),
        Key::Section(Section::Privacy),
        Key::Section(Section::About),
    ];

    pub fn group(self) -> Group {
        match self {
            Key::NewMail | Key::Search | Key::Refresh | Key::Folders | Key::Settings => {
                Group::Window
            }
            Key::Reply
            | Key::ReplyAll
            | Key::Forward
            | Key::Archive
            | Key::Delete
            | Key::MoveTo
            | Key::Important
            | Key::MarkRead
            | Key::MarkUnread
            | Key::SaveEml => Group::Message,
            Key::Send | Key::Attach | Key::Link => Group::Composer,
            Key::Section(_) => Group::Settings,
        }
    }

    /// The letter, upper case, in `lang`. Send is S in both languages, as
    /// the composer's Alt+S has always been (README, Settings > Shortcuts).
    pub fn letter(self, lang: Lang) -> char {
        let (en, pt) = match self {
            Key::NewMail => ('N', 'N'),
            Key::Search => ('E', 'P'),
            Key::Refresh => ('F', 'Z'),
            Key::Folders => ('O', 'T'),
            Key::Settings => ('T', 'F'),
            Key::Reply => ('R', 'R'),
            Key::ReplyAll => ('L', 'O'),
            Key::Forward => ('W', 'H'),
            Key::Archive => ('A', 'A'),
            Key::Delete => ('D', 'L'),
            Key::MoveTo => ('V', 'M'),
            Key::Important => ('I', 'I'),
            Key::MarkRead => ('M', 'C'),
            Key::MarkUnread => ('U', 'D'),
            Key::SaveEml => ('S', 'E'),
            Key::Send => ('S', 'S'),
            Key::Attach => ('A', 'X'),
            Key::Link => ('L', 'G'),
            // Contas, Assinaturas, Regras, Geral, Aspeto, Privacidade,
            // Acerca de: none of the window's letters (P is Search, T
            // Folders). Appearance is C (A, P and E are taken), Aspeto S.
            Key::Section(Section::Accounts) => ('A', 'C'),
            Key::Section(Section::Signatures) => ('S', 'A'),
            Key::Section(Section::Rules) => ('R', 'R'),
            Key::Section(Section::General) => ('G', 'G'),
            Key::Section(Section::Appearance) => ('C', 'S'),
            Key::Section(Section::Privacy) => ('P', 'I'),
            Key::Section(Section::About) => ('B', 'E'),
        };
        match lang {
            Lang::En => en,
            Lang::PtPt => pt,
        }
    }

    /// The action the command's shortcut sends.
    fn action(self) -> Box<dyn gpui::Action> {
        match self {
            Key::NewMail => Box::new(mail::NewMail),
            Key::Search => Box::new(mail::FocusSearch),
            Key::Refresh => Box::new(mail::Refresh),
            Key::Folders => Box::new(mail::ToggleFolders),
            Key::Settings => Box::new(crate::tabs::ToggleSettings),
            Key::Reply => Box::new(mail::Reply),
            Key::ReplyAll => Box::new(mail::ReplyAll),
            Key::Forward => Box::new(mail::Forward),
            Key::Archive => Box::new(mail::Archive),
            Key::Delete => Box::new(mail::Delete),
            Key::MoveTo => Box::new(mail::MoveTo),
            Key::Important => Box::new(mail::ToggleImportant),
            Key::MarkRead => Box::new(mail::MarkRead),
            Key::MarkUnread => Box::new(mail::MarkUnread),
            Key::SaveEml => Box::new(mail::SaveEml),
            Key::Send => Box::new(compose::Send),
            Key::Attach => Box::new(compose::Attach),
            Key::Link => Box::new(compose::OpenLink),
            Key::Section(section) => Box::new(settings::ShowSection(section)),
        }
    }

    /// The keys whose letter in `lang` is `letter` (upper or lower case),
    /// the composer's first, then Settings': where the focus is in the
    /// composer, Alt+A is Attach; in Settings, Accounts; elsewhere Archive.
    pub fn with_letter(letter: char, lang: Lang) -> Vec<Key> {
        let mut keys: Vec<Key> = Key::ALL
            .into_iter()
            .filter(|k| k.letter(lang).eq_ignore_ascii_case(&letter))
            .collect();
        keys.sort_by_key(|k| match k.group() {
            Group::Composer => 0,
            Group::Settings => 1,
            Group::Window | Group::Message => 2,
        });
        keys
    }

    /// What UI Automation is told (`aria-keyshortcuts`).
    pub fn shortcut(self, lang: Lang) -> String {
        format!("Alt+{}", self.letter(lang))
    }
}

/// The bindings are installed once per app.
struct Bound;
impl Global for Bound {}

/// Alt+A to Alt+Z, and the handler that turns a letter into the command.
/// Installed once, by the first control that has an access key
/// ([`decorate`]) or by the app's start-up (`main.rs`).
pub fn bind_keys(cx: &mut App) {
    if cx.has_global::<Bound>() {
        return;
    }
    cx.set_global(Bound);
    for letter in 'a'..='z' {
        cx.bind_keys([KeyBinding::new(
            &format!("alt-{letter}"),
            Press(letter),
            None,
        )]);
    }
    cx.on_action(press);
}

/// A global action handler runs while the window that sent it is busy, so
/// the command is sent from a deferred call.
fn press(action: &Press, cx: &mut App) {
    let (letter, lang) = (action.0, crate::i18n::lang(cx));
    // The window the key was typed in is the active one; a headless test
    // window never is.
    let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) else {
        return;
    };
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, cx| {
            for key in Key::with_letter(letter, lang) {
                let action = key.action();
                if window.is_action_available(action.as_ref(), cx) {
                    window.dispatch_action(action, cx);
                    return;
                }
            }
        });
    });
}

/// Whether the key tips show: Alt is held on its own.
#[derive(Default)]
struct Held(bool);
impl Global for Held {}

/// Whether the access key chips show now.
pub fn shown(cx: &App) -> bool {
    cx.try_global::<Held>().is_some_and(|h| h.0)
}

/// Alt, and nothing else, is down (Ctrl+Alt is AltGr on many keyboards).
fn alt_only(m: &Modifiers) -> bool {
    m.alt && !m.control && !m.shift && !m.platform && !m.function
}

/// Alt is no longer held as far as this app can tell: the window it was
/// pressed in lost the focus, so its release goes elsewhere.
fn release(cx: &mut App) {
    set_held(false, cx);
}

/// Call when a window's activation changes (the main window's observer;
/// [`release_on_deactivate`] for the others): a window that is no longer
/// the active one lets go of Alt, which would otherwise keep every chip
/// drawn until the next modifier change.
pub fn on_activation(window: &Window, cx: &mut App) {
    if !window.is_window_active() {
        release(cx);
    }
}

/// [`on_activation`] for a window root that has no observer of its own.
pub fn release_on_deactivate<V: 'static>(window: &mut Window, cx: &mut Context<V>) {
    cx.observe_window_activation(window, |_, window, cx| on_activation(window, cx))
        .detach();
}

fn set_held(on: bool, cx: &mut App) {
    if shown(cx) != on {
        cx.set_global(Held(on));
        // A refresh also draws the views that are cached.
        cx.refresh_windows();
    }
}

/// The window's root element: shows the key tips while Alt is held. The
/// element must be an ancestor of the focus (the root that holds the
/// window's key context), because modifier changes reach only the focused
/// element's ancestors.
pub trait AccessHost: InteractiveElement + Sized {
    fn access_keys(self) -> Self {
        self.on_modifiers_changed(|event, _, cx| set_held(alt_only(&event.modifiers), cx))
    }
}

impl<T: InteractiveElement> AccessHost for T {}

/// The chip under a control: the letter on an ink pill, as the tooltip
/// draws (no new colour). It hangs below the control's edge, centred.
pub fn chip(letter: char, t: &Tokens) -> impl IntoElement {
    let name = letter.to_ascii_uppercase().to_string();
    div()
        .id(("access-tip", letter.to_ascii_uppercase() as usize))
        .role(Role::Tooltip)
        .aria_label(name.clone())
        .absolute()
        .top_full()
        .left_0()
        .right_0()
        .mt(px(space::SP_0_5))
        .flex()
        .justify_center()
        .child(
            div()
                .px(px(space::SP_1))
                .rounded(px(radius::FULL))
                .bg(t.color.ink)
                .text_color(t.color.on_ink)
                .typo(&t.text.caption)
                .child(name),
        )
        .test_support()
}

/// The chip in a row of a menu the app draws (the keyboard's row menu): the
/// letter on an ink pill at the row's end, as Kit's menus draw theirs.
pub fn inline_chip(letter: char, t: &Tokens) -> impl IntoElement {
    let name = letter.to_ascii_uppercase().to_string();
    div()
        .id(("menu-access-key", letter.to_ascii_uppercase() as usize))
        .role(Role::Tooltip)
        .aria_label(name.clone())
        .flex_none()
        .px(px(space::SP_1))
        .rounded(px(radius::FULL))
        .bg(t.color.ink)
        .text_color(t.color.on_ink)
        .typo(&t.text.caption)
        .child(name)
        .test_support()
}

/// What a control with access key `key` adds to itself: its letter for UI
/// Automation, and the chip while Alt is held. A disabled control does not
/// call this: it has no access key then. The first control with a key also
/// installs the bindings ([`bind_keys`]), so the app needs no start-up line
/// for them.
pub fn decorate(el: gpui::Stateful<Div>, key: Key, cx: &mut App) -> gpui::Stateful<Div> {
    bind_keys(cx);
    let lang = crate::i18n::lang(cx);
    let el = el.aria_keyshortcuts(key.shortcut(lang));
    if shown(cx) {
        let t = cx.tokens().clone();
        el.relative().child(chip(key.letter(lang), &t))
    } else {
        el
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use crate::widgets::button::Button;
    use crate::widgets::icon_button::IconButton;
    use crate::widgets::test_support as widgets;
    use gpui::{
        Capslock, Context, FocusHandle, Focusable, ModifiersChangedEvent, PlatformInput, Render,
        TestAppContext, Window,
    };
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
    use komorebi_i18n::{tr, with_lang};

    /// The names the keys are for, in `lang` (existing catalog messages).
    fn names(key: Key) -> Vec<String> {
        use crate::views::mail::text::{self, Command};
        let mail = |c| text::command(c);
        match key {
            Key::NewMail => vec![text::new_mail()],
            Key::Search => vec![tr!("shortcut" => "Search")],
            Key::Refresh => vec![tr!("Refresh")],
            Key::Folders => vec![text::folders()],
            Key::Settings => vec![crate::views::settings::text::settings_title()],
            Key::Reply => vec![mail(Command::Reply)],
            Key::ReplyAll => vec![mail(Command::ReplyAll)],
            Key::Forward => vec![mail(Command::Forward)],
            Key::Archive => vec![mail(Command::Archive)],
            Key::Delete => vec![mail(Command::Delete)],
            Key::MoveTo => vec![mail(Command::MoveTo)],
            Key::Important => vec![mail(Command::Flag)],
            Key::MarkRead => vec![mail(Command::MarkRead)],
            Key::MarkUnread => vec![mail(Command::MarkUnread)],
            Key::SaveEml => vec![mail(Command::SaveEml)],
            // The composer's Alt+S has always been Send (README).
            Key::Send => vec![],
            Key::Attach => vec![crate::views::composer::text::words::attach()],
            Key::Link => vec![crate::views::composer::text::words::link()],
            Key::Section(section) => vec![section.name()],
        }
    }

    /// A letter without its accent, lower case.
    fn plain(s: &str) -> String {
        s.chars()
            .map(|c| match c.to_ascii_lowercase() {
                'ã' | 'á' | 'à' | 'â' => 'a',
                'é' | 'ê' => 'e',
                'í' => 'i',
                'ó' | 'õ' | 'ô' => 'o',
                'ú' => 'u',
                'ç' => 'c',
                c => c.to_ascii_lowercase(),
            })
            .collect()
    }

    #[test]
    fn keys_that_meet_have_different_letters() {
        for lang in Lang::ALL {
            for (i, a) in Key::ALL.iter().enumerate() {
                for b in &Key::ALL[i + 1..] {
                    if a.group().meets(b.group()) {
                        assert_ne!(
                            a.letter(lang),
                            b.letter(lang),
                            "{a:?} and {b:?} are on screen together but share a letter in {lang:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_letter_is_in_the_name_it_belongs_to() {
        for lang in Lang::ALL {
            with_lang(lang, || {
                for key in Key::ALL {
                    let letter = key.letter(lang).to_ascii_lowercase();
                    assert!(letter.is_ascii_alphabetic(), "{key:?}");
                    for name in names(key) {
                        assert!(
                            plain(&name).contains(letter),
                            "{key:?}: {letter} is not in {name:?} ({lang:?})"
                        );
                    }
                }
            });
        }
    }

    #[test]
    fn the_composer_is_asked_first() {
        // Alt+A in English: Attach where the composer has the focus,
        // Archive in the message pane.
        assert_eq!(
            Key::with_letter('A', Lang::PtPt),
            [Key::Section(Section::Signatures), Key::Archive]
        );
        assert_eq!(Key::with_letter('x', Lang::PtPt), [Key::Attach]);
        assert!(Key::with_letter('z', Lang::En).is_empty());
        assert_eq!(Key::Reply.shortcut(Lang::PtPt), "Alt+R");
        // In Settings its section comes before the message's command.
        assert_eq!(
            Key::with_letter('a', Lang::En),
            [Key::Attach, Key::Section(Section::Accounts), Key::Archive]
        );
        assert_eq!(
            Key::with_letter('s', Lang::En),
            [Key::Send, Key::Section(Section::Signatures), Key::SaveEml]
        );
    }

    /// Alt+S sends in the composer (its own binding) and the access key of
    /// Send is the same letter, so the two never disagree.
    #[test]
    fn send_keeps_alt_s() {
        for lang in Lang::ALL {
            assert_eq!(Key::Send.letter(lang), 'S');
        }
    }

    /// A window root with the commands: records which action ran.
    struct Pane {
        focus: FocusHandle,
        ran: Vec<&'static str>,
        send_disabled: bool,
    }

    impl Focusable for Pane {
        fn focus_handle(&self, _: &App) -> FocusHandle {
            self.focus.clone()
        }
    }

    impl Render for Pane {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("pane")
                .track_focus(&self.focus)
                .access_keys()
                .on_action(cx.listener(|this, _: &mail::Reply, _, _| this.ran.push("reply")))
                .on_action(cx.listener(|this, _: &mail::Archive, _, _| this.ran.push("archive")))
                .on_action(cx.listener(|this, _: &mail::NewMail, _, _| this.ran.push("new")))
                .on_action(cx.listener(|this, _: &mail::FocusSearch, _, _| this.ran.push("search")))
                .flex()
                .gap(px(space::SP_4))
                .p(px(space::SP_8))
                .child(
                    Button::new("new-mail")
                        .label("New mail")
                        .access_key(Key::NewMail)
                        .on_click(cx.listener(|this, _, _, _| this.ran.push("clicked"))),
                )
                .child(
                    IconButton::new("reply", crate::icons::Icon::Reply, "Reply")
                        .access_key(Key::Reply),
                )
                .child(
                    Button::new("search")
                        .label("Search")
                        .access_key(Key::Search),
                )
                .child(
                    Button::new("send")
                        .label("Send")
                        .access_key(Key::Send)
                        .disabled(self.send_disabled),
                )
                .child(Button::new("plain").label("Plain"))
        }
    }

    fn open(
        cx: &mut TestAppContext,
    ) -> (
        gpui::WindowHandle<gpui_kit::component::Root>,
        gpui::Entity<Pane>,
    ) {
        let (window, view) = widgets::open(cx, |window, cx| {
            let pane = Pane {
                focus: cx.focus_handle(),
                ran: Vec::new(),
                send_disabled: true,
            };
            window.focus(&pane.focus, cx);
            pane
        });
        // No `bind_keys` here: the first control with a key installs them.
        cx.update(|cx| crate::i18n::init(Lang::En, cx));
        (window, view)
    }

    fn alt(window: &mut Window, down: bool, cx: &mut App) {
        let modifiers = Modifiers {
            alt: down,
            ..Modifiers::default()
        };
        window.dispatch_event(
            PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                modifiers,
                capslock: Capslock { on: false },
            }),
            cx,
        );
        window.render_frame(cx);
    }

    #[gpui_kit::test]
    fn holding_alt_shows_the_letters_and_alt_and_a_letter_runs_the_command(
        cx: &mut TestAppContext,
    ) {
        let (window, view) = open(cx);
        let window: gpui::AnyWindowHandle = window.into();
        // Nothing shows until Alt is held.
        test_support::in_window(cx, window, |window, _| {
            assert!(window.try_find(("access-tip", 'N' as usize)).is_none());
            assert!(window.try_find_by_label("New mail").is_some());
        });
        // Alt held: a chip under each enabled control with a key.
        cx.update_window(window, |_, window, cx| alt(window, true, cx))
            .unwrap();
        test_support::in_window(cx, window, |window, cx| {
            assert!(shown(cx));
            for letter in ['N', 'R'] {
                assert!(
                    window.try_find(("access-tip", letter as usize)).is_some(),
                    "{letter}"
                );
            }
            assert!(
                window.try_find(("access-tip", 'S' as usize)).is_none(),
                "a disabled control has no key"
            );
        });
        // Alt and the letter: the command's action, as its shortcut sends.
        cx.update_window(window, |_, window, cx| {
            window.press("alt-r", cx);
            window.press("alt-n", cx);
            window.press("alt-a", cx);
            window.press("alt-z", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |v, _| v.ran.clone()),
            ["reply", "new", "archive"]
        );
        // Alt let go: the chips go.
        cx.update_window(window, |_, window, cx| alt(window, false, cx))
            .unwrap();
        test_support::in_window(cx, window, |window, cx| {
            assert!(!shown(cx));
            assert!(window.try_find(("access-tip", 'N' as usize)).is_none());
        });
    }

    #[gpui_kit::test]
    fn ctrl_and_alt_together_is_altgr_and_shows_nothing(cx: &mut TestAppContext) {
        let (window, _) = open(cx);
        let window: gpui::AnyWindowHandle = window.into();
        cx.update_window(window, |_, window, cx| {
            let both = Modifiers {
                alt: true,
                control: true,
                ..Modifiers::default()
            };
            window.dispatch_event(
                PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                    modifiers: both,
                    capslock: Capslock { on: false },
                }),
                cx,
            );
            assert!(!shown(cx));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn the_letters_follow_the_language(cx: &mut TestAppContext) {
        let (window, view) = open(cx);
        let window: gpui::AnyWindowHandle = window.into();
        // Search is E in English and P in Portuguese, so the chip and the
        // key both move with the language.
        cx.update_window(window, |_, window, cx| alt(window, true, cx))
            .unwrap();
        test_support::in_window(cx, window, |window, _| {
            assert!(window.try_find(("access-tip", 'E' as usize)).is_some());
            assert!(window.try_find(("access-tip", 'P' as usize)).is_none());
        });
        cx.update(|cx| crate::i18n::set_language(Lang::PtPt, cx));
        cx.update_window(window, |_, window, cx| {
            alt(window, false, cx);
            alt(window, true, cx);
        })
        .unwrap();
        test_support::in_window(cx, window, |window, _| {
            assert!(window.try_find(("access-tip", 'P' as usize)).is_some());
            assert!(window.try_find(("access-tip", 'E' as usize)).is_none());
            // Reply is R in both languages.
            assert!(window.try_find(("access-tip", 'R' as usize)).is_some());
        });
        cx.update_window(window, |_, window, cx| {
            window.press("alt-e", cx);
            window.press("alt-p", cx);
            window.press("alt-r", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |v, _| v.ran.clone()),
            ["search", "reply"]
        );
    }

    #[gpui_kit::test]
    fn losing_the_window_lets_go_of_alt(cx: &mut TestAppContext) {
        let (window, _) = open(cx);
        let window: gpui::AnyWindowHandle = window.into();
        cx.update_window(window, |_, window, cx| alt(window, true, cx))
            .unwrap();
        cx.update(|cx| assert!(shown(cx)));
        // The window went to the back with Alt down (a headless window is
        // never the active one): the release will reach another window, so
        // the chips go now.
        cx.update_window(window, |_, window, cx| on_activation(window, cx))
            .unwrap();
        cx.update(|cx| assert!(!shown(cx)));
    }
}
