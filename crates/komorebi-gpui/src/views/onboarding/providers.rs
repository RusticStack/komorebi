//! The provider list of the setup's Sign in step: a search box over the
//! built-in providers (`komorebi::providers::PRESETS`), each with its own
//! icon (bundled, `assets/providers`, never fetched), its name and a short
//! note on how it signs in, and "Add provider" for any other server.
//!
//! Keyboard (the composer's suggestion list and Contacts' search do the
//! same): the focus stays in the search box, which filters as you type; Up
//! and Down (Page Up, Page Down, Home, End) move the highlighted provider,
//! which is the list box's selected option so Narrator reads it; Enter
//! picks it; Tab reaches "Add provider"; Esc clears the search, then goes
//! back. A provider Komorebi cannot sign in to (Outlook.com, Microsoft 365)
//! is listed with why, and cannot be picked.

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement, Render, Role, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled, Subscription, Window, div, img, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use komorebi::providers::{self, PRESETS, SignIn};
use komorebi_i18n::tr;

use crate::icons::Icon;
use crate::theme::settings as geo;
use crate::theme::{ActiveTokens as _, TextStyled as _, radius, space};
use crate::widgets::button::{Button, Variant};
use crate::widgets::field::{FieldLook, Nav, TextField};
use crate::widgets::spot::SenderSpot;

/// What the list tells the setup flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerEvent {
    /// A provider was picked (an index into `PRESETS`).
    Picked(usize),
    /// "Add provider": any other server, typed by hand.
    AddProvider,
    /// Esc in an empty search box: the flow goes back.
    Back,
}

macro_rules! provider_icons {
    ($($id:literal,)*) => {
        /// The bundled icons: (file id, 32 px PNG, 64 px PNG).
        const ICONS: &[(&str, &[u8], &[u8])] = &[
            $((
                $id,
                include_bytes!(concat!("../../../../../assets/providers/", $id, ".png")),
                include_bytes!(concat!("../../../../../assets/providers/", $id, "@2x.png")),
            ),)*
        ];
    };
}

provider_icons! {
    "aol", "fastmail", "free", "gmail", "gmx", "google-workspace", "icloud", "infomaniak",
    "libero", "mailboxorg", "mailcom", "orange", "outlook", "postale", "posteo", "proton",
    "purelymail", "seznam", "webde", "yahoo", "yandex", "zoho",
}

/// The asset path prefix the icons are served under (`icons::AppAssets`).
const PREFIX: &str = "komorebi/providers/";

/// The icon file a preset uses: its own, or its sibling's.
fn icon_id(preset_id: &str) -> &str {
    match preset_id {
        "gmx-com" => "gmx",
        "zoho-eu" | "zoho-business" => "zoho",
        "microsoft365" => "outlook",
        other => other,
    }
}

/// The asset path of a preset's icon at 1x or 2x, if one is bundled.
pub fn icon_path(preset_id: &str, double: bool) -> Option<SharedString> {
    let id = icon_id(preset_id);
    ICONS.iter().any(|(i, _, _)| *i == id).then(|| {
        SharedString::from(format!(
            "{PREFIX}{id}{}.png",
            if double { "@2x" } else { "" }
        ))
    })
}

/// The bytes behind an icon's asset path (for `icons::AppAssets`).
pub fn icon_bytes(path: &str) -> Option<&'static [u8]> {
    let file = path.strip_prefix(PREFIX)?.strip_suffix(".png")?;
    let (id, double) = match file.strip_suffix("@2x") {
        Some(id) => (id, true),
        None => (file, false),
    };
    ICONS
        .iter()
        .find(|(i, _, _)| *i == id)
        .map(|(_, one, two)| if double { *two } else { *one })
}

/// Every bundled icon's asset path (for `icons::AppAssets::list`).
pub fn icon_paths() -> impl Iterator<Item = SharedString> {
    ICONS.iter().flat_map(|(id, _, _)| {
        [
            SharedString::from(format!("{PREFIX}{id}.png")),
            SharedString::from(format!("{PREFIX}{id}@2x.png")),
        ]
    })
}

/// The short note under a provider's name: how its accounts sign in.
pub fn note(sign_in: SignIn) -> String {
    match sign_in {
        SignIn::Password => tr!("Your account password"),
        SignIn::AppPasswordIfTwoStep => tr!("App password if two-step sign-in is on"),
        SignIn::AppPassword => tr!("App password needed"),
        SignIn::TurnOnImap => tr!("Turn on IMAP in its settings first"),
        SignIn::TurnOnImapAppPassword => tr!("Turn on IMAP and use an app password"),
        SignIn::PaidImap => tr!("IMAP needs a paid plan"),
        SignIn::Bridge => tr!("Needs Proton Mail Bridge"),
        SignIn::NotSupported => tr!("Not supported yet: needs Microsoft sign-in"),
    }
}

/// A row's accessible name: the provider and its note.
pub fn row_name(index: usize) -> String {
    PRESETS
        .get(index)
        .map_or_else(String::new, |p| tr!("{0}, {1}", p.name, note(p.sign_in)))
}

/// The searchable provider list.
pub struct ProviderPicker {
    search: Entity<InputState>,
    /// The providers the search shows, in list order.
    matches: Vec<usize>,
    /// The highlighted one (an index into `matches`).
    active: usize,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PickerEvent> for ProviderPicker {}

impl ProviderPicker {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr!("Search for your provider")));
        let subscriptions = vec![cx.subscribe(&search, |this: &mut Self, _, e, cx| {
            if matches!(e, InputEvent::Change) {
                let query = this.search.read(cx).value().to_string();
                this.filter(&query, cx);
            }
        })];
        Self {
            search,
            matches: providers::search(""),
            active: 0,
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    /// The search box.
    pub fn search_input(&self) -> &Entity<InputState> {
        &self.search
    }

    /// The providers shown, in order.
    pub fn matches(&self) -> &[usize] {
        &self.matches
    }

    /// The highlighted provider.
    pub fn active(&self) -> Option<usize> {
        self.matches.get(self.active).copied()
    }

    /// Focus the search box.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.search.update(cx, |s, cx| s.focus(window, cx));
    }

    /// Clear the search: the whole list, the first provider highlighted.
    pub fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.set_value("", window, cx));
        self.filter("", cx);
    }

    fn filter(&mut self, query: &str, cx: &mut Context<Self>) {
        self.matches = providers::search(query);
        self.active = 0;
        self.scroll.scroll_to_item(0);
        cx.notify();
    }

    /// Move the highlight (`Nav` from the search box).
    pub fn key(&mut self, key: Nav, window: &mut Window, cx: &mut Context<Self>) {
        let last = self.matches.len().saturating_sub(1);
        let page = 5;
        self.active = match key {
            Nav::Up => self.active.saturating_sub(1),
            Nav::Down => (self.active + 1).min(last),
            Nav::PageUp => self.active.saturating_sub(page),
            Nav::PageDown => (self.active + page).min(last),
            Nav::Home => 0,
            Nav::End => last,
            Nav::Enter | Nav::SecondaryEnter => {
                if let Some(i) = self.active() {
                    self.pick(i, cx);
                }
                return;
            }
            Nav::Escape => {
                if self.search.read(cx).value().is_empty() {
                    cx.emit(PickerEvent::Back);
                } else {
                    self.reset(window, cx);
                }
                return;
            }
        };
        self.scroll.scroll_to_item(self.active);
        cx.notify();
    }

    /// Pick provider `index`, if Komorebi can sign in to it.
    pub fn pick(&mut self, index: usize, cx: &mut Context<Self>) {
        if PRESETS.get(index).is_some_and(|p| p.is_supported()) {
            cx.emit(PickerEvent::Picked(index));
        }
    }
}

impl Render for ProviderPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let double = window.scale_factor() >= 1.5;
        let count = self.matches.len();
        let me = cx.entity().downgrade();
        let rows: Vec<_> = self
            .matches
            .iter()
            .enumerate()
            .map(|(ix, &index)| {
                let p = &PRESETS[index];
                let on = ix == self.active;
                let supported = p.is_supported();
                let icon = match icon_path(p.id, double) {
                    Some(path) => img(path)
                        .size(px(geo::PROVIDER_ICON))
                        .flex_none()
                        .into_any_element(),
                    None => SenderSpot::new(p.name)
                        .size(geo::PROVIDER_ICON)
                        .into_any_element(),
                };
                div()
                    .id(SharedString::from(format!("onboarding-provider-{}", p.id)))
                    .role(Role::ListBoxOption)
                    .aria_label(row_name(index))
                    .aria_selected(on)
                    .aria_position_in_set(ix + 1)
                    .aria_size_of_set(count)
                    .min_h(px(geo::PROVIDER_ROW_H))
                    .flex()
                    .items_center()
                    .gap(px(space::SP_3))
                    .px(px(space::SP_3))
                    .py(px(space::SP_2))
                    .rounded(px(radius::CARD))
                    .when(on, |el| el.bg(c.wash))
                    .when(supported, |el| {
                        el.hover(move |s| s.bg(c.wash)).cursor_pointer()
                    })
                    .child(icon)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .typo(&t.text.body)
                                    .text_color(if supported { c.ink } else { c.ink_3 })
                                    .child(p.name),
                            )
                            .child(
                                div()
                                    .typo(&t.text.caption)
                                    .text_color(c.ink_3)
                                    .child(note(p.sign_in)),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active = ix;
                        this.pick(index, cx);
                    }))
                    .test_support()
            })
            .collect();
        let empty = self.matches.is_empty().then(|| {
            let text = tr!("No provider matches. Use Add provider for any other server.");
            div()
                .id("onboarding-providers-none")
                .role(Role::Status)
                .aria_label(text.clone())
                .p(px(space::SP_3))
                .typo(&t.text.body)
                .text_color(c.ink_2)
                .child(text)
                .test_support()
        });
        div()
            .flex()
            .flex_col()
            .gap(px(space::SP_2))
            .child(
                TextField::new(&self.search, tr!("Search for your provider"))
                    .look(FieldLook::Pill)
                    .leading(Icon::Search)
                    .navigation(move |key, window, cx| {
                        let _ = me.update(cx, |this, cx| this.key(key, window, cx));
                    }),
            )
            .child(
                div()
                    .id("onboarding-providers")
                    .role(Role::ListBox)
                    .aria_label(tr!("Providers"))
                    .max_h(px(geo::PROVIDER_LIST_H))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .gap(px(space::SP_0_5))
                    .children(rows)
                    .children(empty)
                    .test_support(),
            )
            .child(
                div().flex().child(
                    Button::new("onboarding-add-provider")
                        .label(tr!("Add provider"))
                        .name(tr!("Add provider: any other IMAP and SMTP server"))
                        .icon(Icon::Add)
                        .variant(Variant::Secondary)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(PickerEvent::AddProvider))),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every bundled icon is a small PNG of the size its name says, and
    /// every preset has one.
    #[test]
    fn icons_are_small_pngs_for_every_preset() {
        let size = |b: &[u8]| {
            assert!(b.starts_with(b"\x89PNG\r\n\x1a\n"));
            let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
            let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
            (w, h)
        };
        for (id, one, two) in ICONS {
            assert_eq!(size(one), (32, 32), "{id}");
            assert_eq!(size(two), (64, 64), "{id}");
            assert!(one.len() < 4096 && two.len() < 8192, "{id}");
        }
        for p in PRESETS {
            assert!(icon_path(p.id, false).is_some(), "{} has no icon", p.id);
            let path = icon_path(p.id, true).unwrap();
            assert!(icon_bytes(&path).is_some(), "{path}");
        }
        // Not an icon path, or not ours: nothing (no file system access).
        assert!(icon_bytes("komorebi/providers/../../Cargo.toml").is_none());
        assert!(icon_bytes("komorebi/providers/gmail.svg").is_none());
        assert_eq!(icon_paths().count(), ICONS.len() * 2);
    }

    #[test]
    fn notes_and_names_read_in_both_languages() {
        use komorebi::i18n::Lang;
        let gmail = providers::index_of("gmail").unwrap();
        komorebi_i18n::with_lang(Lang::En, || {
            assert_eq!(row_name(gmail), "Gmail, App password needed");
            assert_eq!(note(SignIn::Bridge), "Needs Proton Mail Bridge");
        });
        komorebi_i18n::with_lang(Lang::PtPt, || {
            assert_eq!(row_name(gmail), "Gmail, Requer palavra-passe de aplicação");
        });
    }
}
