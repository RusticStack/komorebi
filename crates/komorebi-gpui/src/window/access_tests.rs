//! Access keys on menu items and the Settings sections (Gaps integration,
//! "Still open"): the letter UI Automation is told, the chip while Alt is
//! held, and the letter running the item. On a throwaway seeded cache.

use gpui::{
    AnyWindowHandle, App, Capslock, Entity, Modifiers, ModifiersChangedEvent, PlatformInput,
    TestAppContext, Window,
};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};

use super::MainWindow;
use crate::test_support::{self, in_window, press};
use crate::views::mail::MailView;
use crate::views::settings::Section;

fn open(
    name: &str,
    cx: &mut TestAppContext,
) -> (
    test_support::TempCache,
    Vec<i64>,
    AnyWindowHandle,
    Entity<MainWindow>,
) {
    let (cache, db, seeded) = test_support::seeded(name, 3);
    db.set_setting("mark_read_delay_ms", "0").expect("setting");
    let ids = db.message_ids(&[seeded.inbox], "").expect("ids");
    test_support::init_app(cx);
    let (window, view) = test_support::open_main(cx, test_support::init(Some(cache.open())));
    (cache, ids, window, view)
}

fn mail(view: &Entity<MainWindow>, cx: &mut TestAppContext) -> Entity<MailView> {
    view.read_with(cx, |v, _| v.mail().cloned().expect("the mail view"))
}

/// Alt pressed or let go, as the platform reports it.
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

/// The letter UI Automation is told for the element named `name`.
fn shortcut_of(window: &Window, name: &str) -> Option<String> {
    window
        .find_by_label(name)
        .keyshortcuts()
        .map(str::to_string)
}

/// The letter UI Automation is told for the menu item named `name`.
fn item_shortcut(window: &Window, name: &str) -> Option<String> {
    window
        .find_all_by_role(gpui::Role::MenuItem)
        .into_iter()
        .find(|e| e.label() == Some(name))
        .unwrap_or_else(|| panic!("no menu item {name}"))
        .keyshortcuts()
        .map(str::to_string)
}

/// Mark as read or unread on the bar, and in a Kit menu (the More menu,
/// where a narrow bar sends it): the bar's button has its access key and
/// chip; the menu item has its letter, its chip while Alt is held, and the
/// letter alone runs it. Save as .eml has its letter in the same menu.
#[gpui_kit::test]
fn menu_items_take_their_access_keys(cx: &mut TestAppContext) {
    let (_cache, ids, window, view) = open("access-menu", cx);
    let mail = mail(&view, cx);
    let m = mail.clone();
    in_window(cx, window, move |w, cx| {
        m.update(cx, |v, cx| v.act(w, cx, |m| m.row_clicked(ids[0])))
    });
    cx.run_until_parked();
    let unread = |cx: &mut TestAppContext| mail.read_with(cx, |v, _| v.mail().row_menu().unread);
    assert!(!unread(cx), "read on opening");
    // The bar's button: Mark as unread, Alt+U, and its chip with Alt held.
    in_window(cx, window, |w, _| {
        assert_eq!(shortcut_of(w, "Mark as unread").as_deref(), Some("Alt+U"));
        assert!(w.try_find(("access-tip", 'U' as usize)).is_none());
    });
    in_window(cx, window, |w, cx| alt(w, true, cx));
    in_window(cx, window, |w, _| {
        assert!(
            w.try_find(("access-tip", 'U' as usize)).is_some(),
            "the bar's chip"
        );
    });
    in_window(cx, window, |w, cx| alt(w, false, cx));

    // Narrow: Mark as unread leaves the bar for More.
    cx.simulate_window_resize(window, gpui::size(gpui::px(420.), gpui::px(800.))); // token-exempt: a one-pane test window
    for _ in 0..4 {
        in_window(cx, window, |_, _| {});
        cx.run_until_parked();
    }
    in_window(cx, window, |w, cx| w.click("cmd-more", cx));
    cx.run_until_parked();
    in_window(cx, window, |w, _| {
        assert_eq!(item_shortcut(w, "Mark as unread").as_deref(), Some("U"));
        assert_eq!(item_shortcut(w, "Save as .eml").as_deref(), Some("S"));
        assert_eq!(item_shortcut(w, "Show original"), None, "no key, no letter");
        assert!(w.try_find(("menu-access-key", 'S' as usize)).is_none());
    });
    // Alt held over the open menu: its items' chips.
    in_window(cx, window, |w, cx| alt(w, true, cx));
    in_window(cx, window, |w, _| {
        for k in ['U', 'S'] {
            assert!(w.try_find(("menu-access-key", k as usize)).is_some(), "{k}");
        }
    });
    in_window(cx, window, |w, cx| alt(w, false, cx));
    // The letter alone runs the item and closes the menu.
    in_window(cx, window, |w, cx| press(w, &["u"], cx));
    cx.run_until_parked();
    in_window(cx, window, |w, _| {
        assert!(
            w.try_find_by_label("Save as .eml").is_none(),
            "the menu closed"
        );
    });
    assert!(unread(cx), "U ran Mark as unread");
}

/// The keyboard's row menu (Shift+F10) takes the same letters as Kit's.
#[gpui_kit::test]
fn the_keyboards_row_menu_takes_the_letters(cx: &mut TestAppContext) {
    let (_cache, ids, window, view) = open("access-flyout", cx);
    let mail = mail(&view, cx);
    let m = mail.clone();
    in_window(cx, window, move |w, cx| {
        m.update(cx, |v, cx| v.act(w, cx, |m| m.row_clicked(ids[0])))
    });
    cx.run_until_parked();
    let focus = mail.read_with(cx, |v, _| v.list_focus().clone());
    in_window(cx, window, |w, cx| {
        w.focus(&focus, cx);
        press(w, &["shift-f10"], cx);
    });
    cx.run_until_parked();
    let unread = |cx: &mut TestAppContext| mail.read_with(cx, |v, _| v.mail().row_menu().unread);
    assert!(!unread(cx));
    in_window(cx, window, |w, _| {
        assert_eq!(item_shortcut(w, "Mark as unread").as_deref(), Some("U"));
        assert_eq!(item_shortcut(w, "Reply").as_deref(), Some("R"));
    });
    in_window(cx, window, |w, cx| alt(w, true, cx));
    in_window(cx, window, |w, _| {
        assert!(
            w.try_find(("menu-access-key", 'U' as usize)).is_some(),
            "the chip"
        );
    });
    in_window(cx, window, |w, cx| alt(w, false, cx));
    in_window(cx, window, |w, cx| press(w, &["u"], cx));
    cx.run_until_parked();
    assert!(unread(cx), "U ran Mark as unread");
}

/// The Settings sections: each has its letter (Alt and it for UI
/// Automation, a chip while Alt is held), and Alt and the letter shows that
/// section, in English and in pt-PT.
#[gpui_kit::test]
fn settings_sections_take_their_access_keys(cx: &mut TestAppContext) {
    let (_cache, _ids, window, view) = open("access-settings", cx);
    in_window(cx, window, |w, cx| press(w, &["ctrl-,"], cx));
    cx.run_until_parked();
    let settings = view
        .read_with(cx, |v, _| v.settings())
        .expect("Settings open");
    let section = |cx: &mut TestAppContext| settings.read_with(cx, |s, _| s.section());
    assert_eq!(section(cx), Section::Accounts);
    in_window(cx, window, |w, _| {
        for (name, keys) in [
            ("Accounts", "Alt+A"),
            ("Signatures", "Alt+S"),
            ("General", "Alt+G"),
            ("Privacy", "Alt+P"),
            ("About", "Alt+B"),
        ] {
            assert_eq!(shortcut_of(w, name).as_deref(), Some(keys), "{name}");
        }
    });
    in_window(cx, window, |w, cx| alt(w, true, cx));
    in_window(cx, window, |w, _| {
        for k in ['A', 'S', 'G', 'P', 'B'] {
            assert!(w.try_find(("access-tip", k as usize)).is_some(), "{k}");
        }
    });
    in_window(cx, window, |w, cx| alt(w, false, cx));
    // Alt+G is General; Alt+A is Accounts here, not Archive.
    in_window(cx, window, |w, cx| press(w, &["alt-g"], cx));
    cx.run_until_parked();
    assert_eq!(section(cx), Section::General);
    in_window(cx, window, |w, cx| press(w, &["alt-a"], cx));
    cx.run_until_parked();
    assert_eq!(section(cx), Section::Accounts);
    let held = mail(&view, cx).read_with(cx, |v, _| v.mail().held().is_some());
    assert!(!held, "nothing archived");
    // pt-PT: Privacidade is Alt+I, Acerca de Alt+E.
    test_support::set_lang(cx, komorebi::i18n::Lang::PtPt);
    in_window(cx, window, |w, _| {
        assert_eq!(shortcut_of(w, "Privacidade").as_deref(), Some("Alt+I"));
        assert_eq!(shortcut_of(w, "Acerca de").as_deref(), Some("Alt+E"));
    });
    in_window(cx, window, |w, cx| press(w, &["alt-i"], cx));
    cx.run_until_parked();
    assert_eq!(section(cx), Section::Privacy);
    test_support::set_lang(cx, komorebi::i18n::Lang::En);
}
