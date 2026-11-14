//! The main window's tabs (docs/GPUI-MIGRATION.md phase 5; the Slint app's
//! `src/app/tabs.rs`, docs/WINUI3.md "TabView"): the fixed Mail tab,
//! message tabs (one message at full width, with its own reader) and draft
//! tabs (one draft each, with its own composer). The model is the
//! library's `komorebi::tabs::Tabs`, shared with the Slint app; the window
//! (`window.rs`) drives it.
//!
//! - `strip.rs`: the strip over the content, from two tabs on (pill tabs on
//!   canvas with the 4 px place spot under the current one, BRAND.md), the
//!   close button, middle-click to close, the tab menu.
//! - `message.rs`: a message tab's view (the command bar over its reader,
//!   the "moved or deleted" notice once the message is gone).
//! - `text.rs`: the words.
//!
//! The keys (`crate::tabs::bind_keys`): Ctrl+Tab and Ctrl+Shift+Tab,
//! Ctrl+PageDown and Ctrl+PageUp, Ctrl+1 to Ctrl+8 and Ctrl+9, Ctrl+W and
//! Ctrl+F4, Ctrl+Shift+T, Ctrl+Shift+PageUp and Ctrl+Shift+PageDown in the
//! window; Shift+Enter (open in a new tab) and Ctrl+O (open in a new
//! window) on the list and the reading pane.

pub mod message;
pub mod strip;
pub mod text;

use komorebi::db::Db;
use komorebi::tabs::{Reopen, Tab, TabKind, Tabs, parse_setting};

/// The `tabs` setting key (ids only, `komorebi::tabs::Tabs::setting`).
pub const SETTING: &str = "tabs";

/// A message's tab title and whether it is unread (none: not cached).
pub fn message_title(db: &Db, id: i64) -> Option<(String, bool)> {
    let (_, subject, seen) = db.subjects(&[id]).ok()?.into_iter().next()?;
    Some((text::title_or_none(subject), !seen))
}

/// The tabs a start brings back from the setting: message tabs whose
/// message is still cached, and saved drafts (loaded when first shown, as
/// `TabKind::Saved`), at most `komorebi::tabs::RESTORE_MAX`; the active
/// one, or the Mail tab when it was not restored.
pub fn restore(db: &Db, value: &str) -> Tabs {
    let (wanted, active) = parse_setting(value);
    let mut tabs = Tabs::default();
    let mut active_at = 0;
    for (i, reopen) in wanted.into_iter().enumerate() {
        let tab = match reopen {
            Reopen::Message(id) => message_title(db, id).map(|(title, unread)| {
                let mut tab = Tab::new(TabKind::Message(id), title);
                tab.unread = unread;
                tab
            }),
            Reopen::Draft(row) => db
                .draft(row)
                .ok()
                .flatten()
                .map(|d| Tab::new(TabKind::Saved(row), text::draft_title(&d.subject))),
        };
        if let Some(tab) = tab {
            let at = tabs.len();
            tabs.insert(at, tab);
            if i + 1 == active {
                active_at = at;
            }
        }
    }
    tabs.set_active(active_at);
    tabs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use komorebi::db::DraftRow;

    /// Port of `tabs_setting_restores_skipping_missing_ids`
    /// (`src/app/composer/tests/tabs_tests.rs`): the setting's message and
    /// draft tabs come back in order; a message or draft no longer cached
    /// is skipped, and the active tab follows its own tab.
    #[test]
    fn tabs_setting_restores_skipping_missing_ids() {
        let (_cache, db, seeded) = test_support::seeded("restore-tabs", 3);
        let ids = db.message_ids(&[seeded.inbox], "").expect("ids");
        let row = db
            .insert_draft(&DraftRow {
                account_id: seeded.account,
                subject: "Plans".into(),
                ..Default::default()
            })
            .expect("a draft");
        let value = format!("m{},m999999,d{row},m{};a=4", ids[0], ids[1]);
        let tabs = restore(&db, &value);
        let kinds: Vec<TabKind> = tabs.iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            [
                TabKind::Mail,
                TabKind::Message(ids[0]),
                TabKind::Saved(row),
                TabKind::Message(ids[1]),
            ]
        );
        assert_eq!(tabs.active(), 3, "the active tab is the same tab");
        assert_eq!(tabs.get(2).map(|t| t.title.as_str()), Some("Plans"));
        let seen = db.subjects(&[ids[0]]).expect("subjects")[0].2;
        assert_eq!(tabs.get(1).map(|t| t.unread), Some(!seen));
        // The skipped one was active: the Mail tab shows.
        let tabs = restore(&db, &format!("m{},m999999;a=2", ids[0]));
        assert_eq!(tabs.active(), 0);
        // Nothing saved: the Mail tab alone.
        assert_eq!(restore(&db, "").len(), 1);
    }
}
