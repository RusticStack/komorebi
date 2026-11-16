//! An account's icon as the views draw it (`komorebi::favicon` decides
//! which): the image the user chose or the domain's, the bundled provider
//! icon, or nothing, in a 20 px slot on the 4 px grid. Dark marks on a clear
//! ground get a neutral backing on the dark theme, and only then.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    AnyElement, Image, ImageFormat, IntoElement, ParentElement, SharedString, Styled, div, img, px,
};
use komorebi::db::Account;
use komorebi::favicon::{self, Shown};

use crate::theme::{Tokens, layout, radius};
use crate::views::Db;
use crate::views::onboarding::providers::icon_path;

/// What an account's slot shows.
#[derive(Clone)]
pub enum Look {
    Image {
        image: Arc<Image>,
        plate: bool,
    },
    /// A bundled provider icon at 1x and 2x.
    Provider {
        one: SharedString,
        two: SharedString,
    },
}

/// Each account's look (accounts with none are left out).
pub type Looks = HashMap<i64, Look>;

/// An account's look, read from the cache.
pub fn look(db: &Db, account: &Account) -> Option<Look> {
    match favicon::shown(db, account) {
        Shown::Image(icon) => Some(Look::Image {
            image: Arc::new(Image::from_bytes(ImageFormat::Png, icon.png)),
            plate: icon.plate,
        }),
        Shown::Provider(id) => Some(Look::Provider {
            one: icon_path(id, false)?,
            two: icon_path(id, true)?,
        }),
        Shown::None => None,
    }
}

/// Every account's look.
pub fn looks(db: &Db, accounts: &[Account]) -> Looks {
    accounts
        .iter()
        .filter_map(|a| look(db, a).map(|l| (a.id, l)))
        .collect()
}

/// The 20 px slot (`double`: the 2x asset, from 150% up). On the dark
/// theme a dark mark on clear sits on a neutral round backing.
pub fn slot(look: &Look, double: bool, t: &Tokens) -> AnyElement {
    // The backing is a spot (BRANDING.md: a circle), the mark inside it.
    let plate = matches!(look, Look::Image { plate: true, .. }) && t.scheme.is_dark();
    let inner = px(if plate { layout::ICON } else { layout::ICON_LG });
    let picture = match look {
        Look::Image { image, .. } => img(image.clone()).size(inner).into_any_element(),
        Look::Provider { one, two } => img(if double { two.clone() } else { one.clone() })
            .size(inner)
            .into_any_element(),
    };
    div()
        .flex_none()
        .size(px(layout::ICON_LG))
        .flex()
        .items_center()
        .justify_center()
        .when(plate, |el| el.rounded(px(radius::FULL)).bg(t.color.ink_2))
        .child(picture)
        .into_any_element()
}

use gpui::prelude::*;
