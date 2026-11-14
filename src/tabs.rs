//! The main window's tabs as state (docs/WINUI3.md "TabView"): the fixed
//! Mail tab (the folders, the list and the reading pane), message tabs (one
//! message at full width) and composer tabs (one draft each). No toolkit
//! here: the Slint app (`src/app/tabs.rs`) and the GPUI app drive the same
//! model, and its tests are plain `#[test]`s (docs/GPUI-TESTING.md).
//!
//! The `tabs` setting holds ids only (message ids, draft rows and the
//! active tab), never a subject, an address or any text.

use std::collections::VecDeque;

/// At most this many tabs come back at a start.
pub const RESTORE_MAX: usize = 20;
/// Ctrl+Shift+T reopens up to this many closed tabs (this session only).
const CLOSED_MAX: usize = 10;

/// A draft held in memory, numbered by the app's composer for this session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DraftId(u64);

impl DraftId {
    pub fn new(n: u64) -> Self {
        DraftId(n)
    }
}

/// What a tab shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabKind {
    /// The folders, the list and the reading pane; always the first tab.
    Mail,
    /// One message, by its cache id.
    Message(i64),
    /// One draft held in memory.
    Draft(DraftId),
    /// A saved draft row not loaded yet (a restored tab): it loads when
    /// the tab is first shown.
    Saved(i64),
}

impl TabKind {
    /// The kind the window shows: 0 Mail, 1 a message, 2 a draft.
    pub fn code(self) -> i32 {
        match self {
            TabKind::Mail => 0,
            TabKind::Message(_) => 1,
            TabKind::Draft(_) | TabKind::Saved(_) => 2,
        }
    }
}

/// A closed tab, as Ctrl+Shift+T opens it again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reopen {
    Message(i64),
    /// A saved draft row.
    Draft(i64),
}

/// How a message showed in a tab, put back when the tab is shown again.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewState {
    /// The reader's scroll (0 or less).
    pub scroll_y: f32,
    /// "View as plain text".
    pub plain: bool,
    /// "Show original colours".
    pub original: bool,
}

#[derive(Clone, Debug)]
pub struct Tab {
    pub kind: TabKind,
    /// The subject (or "New message"); only in the window's model, never
    /// in the setting or a log.
    pub title: String,
    pub unread: bool,
    /// The message was moved or deleted on the server: it stays readable
    /// from the cache until the tab closes, its actions off.
    pub gone: bool,
    pub view: ViewState,
}

impl Tab {
    pub fn new(kind: TabKind, title: String) -> Self {
        Tab {
            kind,
            title,
            unread: false,
            gone: false,
            view: ViewState::default(),
        }
    }
}

/// What the Mail tab's reading pane held when another tab took over.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MailPane {
    pub open: Option<i64>,
    pub draft: Option<DraftId>,
    pub view: ViewState,
}

pub struct Tabs {
    list: Vec<Tab>,
    active: usize,
    closed: VecDeque<Reopen>,
    /// The Mail tab's pane while another tab shows.
    pub mail: MailPane,
}

impl Default for Tabs {
    fn default() -> Self {
        Tabs {
            list: vec![Tab::new(TabKind::Mail, String::new())],
            active: 0,
            closed: VecDeque::new(),
            mail: MailPane::default(),
        }
    }
}

impl Tabs {
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Never: the Mail tab is always there.
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn active_kind(&self) -> TabKind {
        self.list[self.active].kind
    }

    pub fn active_tab(&self) -> &Tab {
        &self.list[self.active]
    }

    pub fn active_tab_mut(&mut self) -> &mut Tab {
        &mut self.list[self.active]
    }

    pub fn get(&self, i: usize) -> Option<&Tab> {
        self.list.get(i)
    }

    pub fn get_mut(&mut self, i: usize) -> Option<&mut Tab> {
        self.list.get_mut(i)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Tab> {
        self.list.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Tab> {
        self.list.iter_mut()
    }

    pub fn find(&self, kind: TabKind) -> Option<usize> {
        self.list.iter().position(|t| t.kind == kind)
    }

    /// The tab of message `id`, if one is open.
    pub fn find_message(&self, id: i64) -> Option<usize> {
        self.find(TabKind::Message(id))
    }

    /// Make tab `i` the active one (the caller saves and shows).
    pub fn set_active(&mut self, i: usize) {
        if i < self.list.len() {
            self.active = i;
        }
    }

    /// Put `tab` at `at` (after the Mail tab at least, at the end at
    /// most); returns its index. The active tab stays the same tab.
    pub fn insert(&mut self, at: usize, tab: Tab) -> usize {
        let at = at.clamp(1, self.list.len());
        self.list.insert(at, tab);
        if at <= self.active {
            self.active += 1;
        }
        at
    }

    /// Remove tab `i` (never the Mail tab). When it was the active one, the
    /// tab to its left becomes active (the caller shows it).
    pub fn remove(&mut self, i: usize) -> Option<Tab> {
        if i == 0 || i >= self.list.len() {
            return None;
        }
        let tab = self.list.remove(i);
        if i < self.active || (i == self.active && self.active > 0) {
            self.active -= 1;
        }
        Some(tab)
    }

    /// Remember a closed tab for Ctrl+Shift+T.
    pub fn remember(&mut self, reopen: Reopen) {
        self.closed.retain(|r| *r != reopen);
        self.closed.push_back(reopen);
        while self.closed.len() > CLOSED_MAX {
            self.closed.pop_front();
        }
    }

    /// The last closed tab, taken off the stack.
    pub fn take_closed(&mut self) -> Option<Reopen> {
        self.closed.pop_back()
    }

    pub fn has_closed(&self) -> bool {
        !self.closed.is_empty()
    }

    /// Move tab `from` to place `to`; the Mail tab stays first. True if it
    /// moved. The active tab stays the same tab.
    pub fn move_tab(&mut self, from: usize, to: usize) -> bool {
        let n = self.list.len();
        if from == 0 || from >= n {
            return false;
        }
        let to = to.clamp(1, n - 1);
        if to == from {
            return false;
        }
        let active = self.list[self.active].kind;
        let tab = self.list.remove(from);
        self.list.insert(to, tab);
        self.active = self.find(active).unwrap_or(0);
        true
    }

    /// The tab `delta` places from the active one, round the strip.
    pub fn step(&self, delta: isize) -> usize {
        let n = self.list.len() as isize;
        (self.active as isize + delta).rem_euclid(n) as usize
    }

    /// Ctrl+1..8 go to tab N (when there is one), Ctrl+9 to the last.
    pub fn number(&self, n: usize) -> Option<usize> {
        match n {
            9 => Some(self.list.len() - 1),
            1..=8 => (n - 1 < self.list.len()).then_some(n - 1),
            _ => None,
        }
    }

    /// The `tabs` setting: `m<message id>` and `d<draft row>` tokens in
    /// order, comma separated, then `;a=<index>` of the active one among
    /// them (the Mail tab is implied, 0). A draft without a saved row is
    /// left out (there is nothing to open again).
    pub fn setting(&self, draft_row: impl Fn(DraftId) -> Option<i64>) -> String {
        let mut tokens = Vec::new();
        let mut active = 0;
        for (i, tab) in self.list.iter().enumerate().skip(1) {
            let token = match tab.kind {
                TabKind::Message(id) => Some(format!("m{id}")),
                TabKind::Draft(d) => draft_row(d).map(|row| format!("d{row}")),
                TabKind::Saved(row) => Some(format!("d{row}")),
                TabKind::Mail => None,
            };
            if let Some(token) = token {
                tokens.push(token);
                if i == self.active {
                    active = tokens.len();
                }
            }
        }
        if tokens.is_empty() {
            return String::new();
        }
        format!("{};a={active}", tokens.join(","))
    }
}

/// The `tabs` setting back ([`Tabs::setting`]): the tabs in order, at most
/// [`RESTORE_MAX`], and the active one's index (0: the Mail tab). Unknown
/// or broken tokens are skipped.
pub fn parse_setting(value: &str) -> (Vec<Reopen>, usize) {
    let (list, active) = value.split_once(";a=").unwrap_or((value, "0"));
    let mut tabs: Vec<Reopen> = Vec::new();
    let mut map = Vec::new();
    for token in list.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let number = || token.get(1..)?.parse::<i64>().ok().filter(|&n| n > 0);
        let parsed = match token.get(..1) {
            Some("m") => number().map(Reopen::Message),
            Some("d") => number().map(Reopen::Draft),
            _ => None,
        };
        map.push(parsed.map(|p| {
            if let Some(i) = tabs.iter().position(|t| *t == p) {
                i + 1
            } else if tabs.len() < RESTORE_MAX {
                tabs.push(p);
                tabs.len()
            } else {
                0
            }
        }));
    }
    let active = active
        .trim()
        .parse::<usize>()
        .ok()
        .and_then(|a| a.checked_sub(1))
        .and_then(|a| map.get(a).copied().flatten())
        .unwrap_or(0);
    (tabs, active)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(kinds: &[TabKind]) -> Tabs {
        let mut t = Tabs::default();
        for &k in kinds {
            let n = t.len();
            t.insert(n, Tab::new(k, String::new()));
        }
        t
    }

    fn kinds(t: &Tabs) -> Vec<TabKind> {
        t.iter().map(|x| x.kind).collect()
    }

    #[test]
    fn close_tab_activates_left_neighbour() {
        let mut t = tabs(&[
            TabKind::Message(1),
            TabKind::Message(2),
            TabKind::Message(3),
        ]);
        t.set_active(2);
        assert_eq!(t.remove(2).unwrap().kind, TabKind::Message(2));
        assert_eq!(t.active_kind(), TabKind::Message(1));
        // A tab left of the active one: the active tab stays.
        t.set_active(2);
        t.remove(1);
        assert_eq!(t.active_kind(), TabKind::Message(3));
        // The Mail tab never closes.
        assert!(t.remove(0).is_none());
        t.remove(1);
        assert_eq!(t.active_kind(), TabKind::Mail);
        assert_eq!(t.len(), 1);
    }

    /// The model half of the Slint app's view test
    /// `close_tab_activates_left_neighbour_and_ctrl_shift_t_reopens`
    /// (`src/app/composer/tests/tabs_tests.rs`), ported to the state struct
    /// (docs/GPUI-TEST-PORT.md): what Ctrl+W, Ctrl+Shift+T and
    /// Ctrl+Shift+PageUp/PageDown do to the tabs, without a window. The
    /// reader's subject after the switch stays with the view test.
    #[test]
    fn close_then_reopen_and_move_as_the_tab_keys_do() {
        let (second, third) = (2, 3);
        let mut t = tabs(&[TabKind::Message(second), TabKind::Message(third)]);
        t.set_active(2);
        // Ctrl+W: the active tab closes and is remembered; the left
        // neighbour shows.
        let closed = t.remove(t.active()).expect("a message tab closes");
        assert_eq!(closed.kind, TabKind::Message(third));
        t.remember(Reopen::Message(third));
        assert_eq!(kinds(&t), [TabKind::Mail, TabKind::Message(second)]);
        assert_eq!(t.active(), 1);
        // Ctrl+Shift+T: the last closed tab comes back at the end, active.
        let Some(Reopen::Message(id)) = t.take_closed() else {
            panic!("a closed message tab to reopen")
        };
        let n = t.len();
        let at = t.insert(n, Tab::new(TabKind::Message(id), String::new()));
        t.set_active(at);
        assert_eq!(
            kinds(&t),
            [
                TabKind::Mail,
                TabKind::Message(second),
                TabKind::Message(third)
            ]
        );
        assert_eq!(t.active(), 2);
        assert!(!t.has_closed());
        // Ctrl+1 then Ctrl+W: the Mail tab never closes.
        t.set_active(t.number(1).expect("the Mail tab"));
        assert!(t.remove(t.active()).is_none());
        assert_eq!(t.len(), 3);
        // Ctrl+Shift+PageUp on the first message tab: the Mail tab stays
        // first; Ctrl+Shift+PageDown moves it right and it stays active.
        t.set_active(1);
        assert!(!t.move_tab(1, 0));
        assert_eq!(kinds(&t)[0], TabKind::Mail);
        assert!(t.move_tab(1, 2));
        assert_eq!(
            kinds(&t),
            [
                TabKind::Mail,
                TabKind::Message(third),
                TabKind::Message(second)
            ]
        );
        assert_eq!(t.active(), 2);
    }

    #[test]
    fn reorder_keeps_mail_tab_first() {
        let mut t = tabs(&[
            TabKind::Message(1),
            TabKind::Message(2),
            TabKind::Message(3),
        ]);
        t.set_active(1);
        assert!(!t.move_tab(0, 2));
        assert!(!t.move_tab(1, 0), "never before the Mail tab");
        assert_eq!(t.get(0).unwrap().kind, TabKind::Mail);
        assert_eq!(t.get(1).unwrap().kind, TabKind::Message(1));
        assert!(t.move_tab(1, 3));
        assert_eq!(
            kinds(&t),
            [
                TabKind::Mail,
                TabKind::Message(2),
                TabKind::Message(3),
                TabKind::Message(1)
            ]
        );
        // The active tab is still message 1.
        assert_eq!(t.active_kind(), TabKind::Message(1));
        // Inserting before the active tab keeps it active.
        t.insert(1, Tab::new(TabKind::Message(9), String::new()));
        assert_eq!(t.active_kind(), TabKind::Message(1));
    }

    #[test]
    fn steps_and_numbers() {
        let t = tabs(&[TabKind::Message(1), TabKind::Message(2)]);
        assert_eq!(t.step(1), 1);
        assert_eq!(t.step(-1), 2);
        assert_eq!(t.number(1), Some(0));
        assert_eq!(t.number(3), Some(2));
        assert_eq!(t.number(4), None);
        assert_eq!(t.number(9), Some(2));
    }

    #[test]
    fn closed_tabs_stack_up_to_ten() {
        let mut t = Tabs::default();
        for i in 1..=12 {
            t.remember(Reopen::Message(i));
        }
        t.remember(Reopen::Message(5));
        assert_eq!(t.take_closed(), Some(Reopen::Message(5)));
        assert_eq!(t.take_closed(), Some(Reopen::Message(12)));
        let mut n = 2;
        while t.take_closed().is_some() {
            n += 1;
        }
        assert_eq!(n, 10);
    }

    #[test]
    fn tabs_setting_holds_ids_only() {
        let mut t = tabs(&[
            TabKind::Message(123),
            TabKind::Draft(DraftId::new(1)),
            TabKind::Draft(DraftId::new(2)),
            TabKind::Saved(7),
        ]);
        t.get_mut(1).unwrap().title = "Lunch with Ana <ana@example.org>".into();
        t.set_active(4);
        // Draft 2 has no saved row yet: left out, and the index follows.
        let s = t.setting(|d| (d == DraftId::new(1)).then_some(45));
        assert_eq!(s, "m123,d45,d7;a=3");
        assert!(!s.contains("Lunch") && !s.contains('@'));
        assert_eq!(
            parse_setting(&s),
            (
                vec![Reopen::Message(123), Reopen::Draft(45), Reopen::Draft(7)],
                3
            )
        );
        // Only the Mail tab: nothing to keep.
        assert_eq!(Tabs::default().setting(|_| None), "");
    }

    #[test]
    fn restore_skips_broken_tokens_and_caps_at_20() {
        let many: Vec<String> = (1..=30).map(|i| format!("m{i}")).collect();
        let (list, active) = parse_setting(&format!("{},x9,m-2,d,m3;a=30", many.join(",")));
        assert_eq!(list.len(), RESTORE_MAX);
        assert_eq!(list[0], Reopen::Message(1));
        // The active one was cut: the Mail tab.
        assert_eq!(active, 0);
        assert_eq!(
            parse_setting("m5,junk,d6;a=3"),
            (vec![Reopen::Message(5), Reopen::Draft(6)], 2)
        );
        assert_eq!(parse_setting(""), (Vec::new(), 0));
        assert_eq!(parse_setting("m1;a=zz"), (vec![Reopen::Message(1)], 0));
        assert_eq!(parse_setting("é5,m2"), (vec![Reopen::Message(2)], 0));
    }
}
