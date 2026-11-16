//! The app-password help (`providers::AppPassword`): Gmail and the other
//! providers that turn the usual password away over IMAP and SMTP. The
//! password field is called "App password", a line under it says why, and
//! a button opens the provider's page in the browser (through the link
//! prompt, only when pressed). After a refused sign-in the same words and
//! button replace the bare "wrong password". The onboarding flow and the
//! account page share these.

use gpui::{
    App, Div, InteractiveElement as _, IntoElement, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use komorebi::providers::AppPassword;
use komorebi_i18n::tr;

use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, space};
use crate::widgets::button::{Button, Variant};

/// The password field's label: "App password" where it always is one,
/// else the usual "Password" (Zoho: only with two-step sign-in on).
pub fn field_label(help: &AppPassword) -> String {
    if help.always() {
        tr!("App password")
    } else {
        tr!("Password")
    }
}

/// The field's placeholder where it is always an app password.
pub fn placeholder() -> String {
    tr!("Paste your app password")
}

/// The line under the field, before any try.
pub fn needs(help: &AppPassword) -> String {
    if help.always() {
        tr!(
            "{0} needs an app password instead of your usual one.",
            help.brand
        )
    } else {
        tr!(
            "If two-step sign-in is on, {0} needs an app password.",
            help.brand
        )
    }
}

/// A quiet line under the field that Narrator reads too.
pub fn line(id: &str, text: String, cx: &App) -> impl IntoElement + use<> {
    let t = cx.tokens();
    div()
        .id(SharedString::from(format!("{id}-app-password-line")))
        .role(Role::Label)
        .aria_label(text.clone())
        .typo(&t.text.caption)
        .text_color(t.color.ink_3)
        .child(text)
        .test_support()
}

/// A refused sign-in, as the notice says it (a title and a sentence).
pub fn refused(help: &AppPassword) -> (String, String) {
    (
        tr!("{0} didn't accept that", help.brand),
        if help.always() {
            tr!("It needs an app password, not your usual password.")
        } else {
            tr!("If two-step sign-in is on, it needs an app password.")
        },
    )
}

/// The button that opens the page where an app password is made.
pub fn create_button(
    id: &str,
    help: &AppPassword,
    open: impl Fn(String, &mut Window, &mut App) + 'static,
) -> Button {
    let url = help.create.to_string();
    Button::new(SharedString::from(format!("{id}-create-app-password")))
        .label(tr!("Create app password"))
        .icon(Icon::Open)
        .on_click(move |_, window, cx| open(url.clone(), window, cx))
}

/// The buttons under the field: Create app password and, where the
/// provider asks for two-step sign-in first (Google), a quieter link to
/// turn it on.
pub fn links(
    id: &str,
    help: &AppPassword,
    open: impl Fn(String, &mut Window, &mut App) + Clone + 'static,
) -> Div {
    let two_step = help.two_step.map(|url| {
        let open = open.clone();
        let url = url.to_string();
        Button::new(SharedString::from(format!("{id}-two-step")))
            .label(tr!("Turn on 2-Step Verification first"))
            .variant(Variant::Subtle)
            .on_click(move |_, window, cx| open(url.clone(), window, cx))
    });
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(space::SP_2))
        .child(create_button(id, help, open))
        .children(two_step)
}
