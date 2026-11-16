//! The app's views (docs/GPUI-MIGRATION.md phases 2 to 10). Each lane adds
//! its module here, one line each.
//!
//! The mail view (`mail`, phase 2) lays out the folder pane, the list and
//! the reading pane; it mounts the reader (`reader`, phase 3) and the
//! composer (`composer`, phase 4) through one typed constructor each,
//! `new(db: Db, window, cx)`, so the three lanes never edit each other's
//! files.

use std::rc::Rc;

pub mod account_icon;
pub mod composer;
pub mod contacts;
pub mod mail;
pub mod onboarding;
pub mod reader;
pub mod rules;
pub mod settings;
pub mod tabs;
pub mod updates;
pub mod windows;

/// The UI thread's cache connection, shared by every view of a window
/// (opened once at start, never on another thread).
pub type Db = Rc<komorebi::db::Db>;
