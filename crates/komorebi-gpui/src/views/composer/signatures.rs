//! The Signature menu (the Slint app's `src/app/signatures.rs`, composer
//! half): every named signature (the one in the message checked), None,
//! then "Manage signatures…" (Settings > Signatures, phase 6). A pick
//! replaces the signature in the message, even an edited one (Ctrl+Z puts
//! it back in a formatted message): `Draft::pick_signature`.

use gpui::{Context, IntoElement, Window};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use komorebi::compose;

use super::{ComposerEvent, ComposerView, text};
use crate::icons::Icon;
use crate::widgets::button::IconButton;
use crate::widgets::menu;

impl ComposerView {
    /// Which signature the message holds: its id (none: no signature, or
    /// one that is not a named signature any more).
    pub(crate) fn current_signature(&self, cx: &gpui::App) -> Option<i64> {
        if let Some(ed) = self.body.read(cx).editor() {
            return ed.signature().and_then(|s| s.source);
        }
        let d = self.drafts.active()?;
        let block = d.signature.as_deref().filter(|b| !b.is_empty())?;
        self.db()
            .signatures()
            .ok()?
            .into_iter()
            .find(|s| {
                self.db()
                    .signature(s.id)
                    .ok()
                    .flatten()
                    .is_some_and(|x| compose::signature_block(&x.body_text) == block)
            })
            .map(|s| s.id)
    }

    /// A pick from the menu: signature `id` (none: no signature).
    pub(crate) fn pick_signature(
        &mut self,
        id: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sig = id.and_then(|id| self.db().signature(id).ok().flatten());
        let changed = self.lend(cx, |d| d.pick_signature(sig.as_ref()));
        if changed == Some(true) {
            self.refresh_body(window, cx);
            self.edited(cx);
        }
    }

    pub(crate) fn signature_button(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let list = self.db().signatures().unwrap_or_default();
        let current = self.current_signature(cx);
        let view = cx.entity().downgrade();
        IconButton::new(
            "compose-signature",
            Icon::Signature,
            text::words::signature(),
        )
        .small()
        .dropdown_menu(move |m, _, cx| {
            let mut m = menu::brand(m, cx);
            for s in &list {
                let (v, id) = (view.clone(), s.id);
                m = m.item(
                    PopupMenuItem::new(s.name.clone())
                        .checked(current == Some(id))
                        .on_click(move |_, window, cx| {
                            let _ =
                                v.update(cx, |this, cx| this.pick_signature(Some(id), window, cx));
                        }),
                );
            }
            let none = view.clone();
            let manage = view.clone();
            m.item(
                PopupMenuItem::new(text::words::signature_none())
                    .checked(current.is_none())
                    .on_click(move |_, window, cx| {
                        let _ = none.update(cx, |this, cx| this.pick_signature(None, window, cx));
                    }),
            )
            .separator()
            .item(
                PopupMenuItem::new(text::words::manage_signatures()).on_click(move |_, _, cx| {
                    let _ = manage.update(cx, |_, cx| cx.emit(ComposerEvent::ManageSignatures));
                }),
            )
        })
    }
}
