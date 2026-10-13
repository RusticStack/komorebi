//! The text Rust shows in the UI. All of it lives in `ui/tr.slint` (the one
//! catalog the `.slint` files use, `translations/<tag>/LC_MESSAGES/
//! komorebi.po`): a `Tr` function per sentence, whole, with its names and
//! numbers as arguments. This module reaches them from anywhere on the UI
//! thread: `t!(invoke_say_lost, host.into())`.
//!
//! Which sentence a typed value (an error, a validation failure) means is
//! decided in `app/text.rs`; the library produces no sentences.

use crate::{AppWindow, Tr};
use slint::{ComponentHandle, Weak};
use std::cell::RefCell;

thread_local! {
    static UI: RefCell<Option<Weak<AppWindow>>> = const { RefCell::new(None) };
}

/// Make `ui`'s `Tr` the one [`with`] and [`t!`] read (the window must exist
/// for the translations to: Slint bundles them when it is created).
pub fn register(ui: &AppWindow) {
    UI.with(|u| *u.borrow_mut() = Some(ui.as_weak()));
}

/// Run `f` on the registered window's `Tr`, in the language shown now.
/// `None` when no window is registered (or it is gone).
pub fn with<R>(f: impl FnOnce(Tr<'_>) -> R) -> Option<R> {
    let ui = UI.with(|u| u.borrow().as_ref().and_then(Weak::upgrade))?;
    Some(f(ui.global::<Tr>()))
}

/// One sentence of `Tr` as a `String`: `t!(invoke_name, args...)`. Empty if
/// no window is registered.
macro_rules! t {
    ($f:ident $(, $a:expr)* $(,)?) => {
        $crate::tr::with(|tr| tr.$f($($a),*).to_string()).unwrap_or_default()
    };
}
pub(crate) use t;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{Platform, WindowAdapter};
    use std::rc::Rc;

    struct TestPlatform(Rc<MinimalSoftwareWindow>);

    impl Platform for TestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            Ok(self.0.clone())
        }
    }

    /// An app window on a headless platform (for the test's own thread),
    /// shown in English and registered for [`t!`]. For tests of text; the
    /// `Harness`es of the composer tests bring their own.
    pub(crate) fn window() -> AppWindow {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        // Once per thread; a test thread has had none yet.
        slint::platform::set_platform(Box::new(TestPlatform(window))).ok();
        let ui = AppWindow::new().expect("a window");
        register(&ui);
        assert!(komorebi::i18n::select(komorebi::i18n::Lang::En));
        ui
    }
}
