//! The list pane (mockup `contacts-*`, left pane): "Contacts" and the
//! count, New contact and Sync (Address book sync), the search
//! field, the group filter once there are groups, then a row per contact
//! under its letter's header in a virtual list (avatar, name, address), or
//! what an empty list says. Once the list scrolls, the letter of the rows
//! at the top stays over it (a sticky header).
//! The list is one tab stop: Up, Down, Home, End, Page Up and Page Down
//! move the contact shown, Enter opens it in one-pane layout.

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Pixels,
    Role, ScrollStrategy, Styled, Window, canvas, div, point, prelude::*, px, size,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::v_virtual_list;
use komorebi::db::contacts::ContactRow;

use super::{ContactsView, text};
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, Tokens, layout, space};
use crate::widgets::button::IconButton;
use crate::widgets::field::{Nav, TextField};
use crate::widgets::pane::ListRow;
use crate::widgets::select::Dropdown;
use crate::widgets::spot::SenderSpot;

/// Rows a Page Up or Page Down moves.
const PAGE_ROWS: usize = 8;

pub fn render(
    view: &mut ContactsView,
    window: &mut Window,
    cx: &mut Context<ContactsView>,
) -> AnyElement {
    let t = cx.tokens().clone();
    let body = if view.page.rows.is_empty() {
        empty(view, &t)
    } else {
        rows(view, window, cx)
    };
    div()
        .size_full()
        .min_h_0()
        .flex()
        .flex_col()
        .child(header(view, &t, cx))
        .child(tools(view, cx))
        .child(body)
        .into_any_element()
}

fn header(view: &ContactsView, t: &Tokens, cx: &mut Context<ContactsView>) -> AnyElement {
    let add = cx.listener(|this, _, window, cx| this.new_contact(window, cx));
    let sync = cx.listener(|this, _, window, cx| this.open_sync(window, cx));
    div()
        .flex_none()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(space::SP_1))
        .pt(px(space::SP_4 + space::SP_0_5))
        .pl(px(space::SP_5))
        .pr(px(space::SP_3))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .id("contacts-title")
                        .role(Role::Heading)
                        .aria_label(text::contacts())
                        .typo(&t.text.subtitle)
                        .text_color(t.color.ink)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(text::contacts())
                        .test_support(),
                )
                .child(
                    div()
                        .typo(&t.text.caption)
                        .text_color(t.color.ink_3)
                        .child(text::count(view.page.total())),
                ),
        )
        .child(IconButton::new("contacts-new", Icon::PersonAdd, text::new_contact()).on_click(add))
        // Address book sync is a visible button, as the Slint app's "Sync"
        // beside New contact (one press, not a menu of one item).
        .child(
            IconButton::new("contacts-sync", Icon::Sync, text::sync())
                .tooltip(text::sync_title())
                .on_click(sync),
        )
        .into_any_element()
}

/// The search field and the group filter.
fn tools(view: &ContactsView, cx: &mut Context<ContactsView>) -> AnyElement {
    let entity = cx.entity().downgrade();
    let nav = entity.clone();
    let groups = (!view.page.groups.is_empty()).then(|| {
        let mut choices = vec![("0".to_string(), text::all_contacts())];
        choices.extend(
            view.page
                .groups
                .iter()
                .enumerate()
                .map(|(i, g)| ((i + 1).to_string(), g.name.clone())),
        );
        Dropdown::new(
            "contacts-group",
            text::groups(),
            view.page.group_index().to_string(),
        )
        .choices(choices)
        .on_pick(move |v, _, cx| {
            if let Ok(i) = v.parse::<usize>() {
                let _ = entity.update(cx, |this, cx| this.pick_group(i, cx));
            }
        })
    });
    div()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(space::SP_2))
        .px(px(space::SP_4))
        .pt(px(space::SP_3))
        .pb(px(space::SP_2))
        .child(
            TextField::new(&view.search, text::search())
                .leading(Icon::Search)
                // Down and Enter hand over to the list; Esc clears.
                .navigation(move |key, window, cx| {
                    let _ = nav.update(cx, |this, cx| this.search_key(key, window, cx));
                }),
        )
        .children(groups.map(|g| div().flex().child(g)))
        .into_any_element()
}

fn empty(view: &ContactsView, t: &Tokens) -> AnyElement {
    let said = if view.page.query.is_empty() {
        text::none_yet()
    } else {
        text::none_match()
    };
    div()
        .id("contacts-empty")
        .role(Role::Status)
        .aria_label(said.clone())
        .flex_1()
        .px(px(space::SP_5))
        .pt(px(space::SP_4))
        .typo(&t.text.body)
        .text_color(t.color.ink_2)
        .child(said)
        .test_support()
        .track_focus(&view.list_focus)
        .into_any_element()
}

/// One entry of the virtual list: a letter header (the mockup's `.letter`)
/// or a contact row (its position in `page.rows`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Item {
    Letter(char),
    Row(usize),
}

/// The list's entries: a letter header before the first row of each
/// letter, then the rows. The library lists each letter's rows together
/// (`komorebi::contacts::list_order`, the order `Db::contact_ids` returns),
/// so each letter has one header; should a letter come back later anyway,
/// it gets a second header rather than rows under the wrong letter.
pub(crate) fn items(rows: &[ContactRow]) -> Vec<Item> {
    let mut out = Vec::with_capacity(rows.len() + 32);
    for (ix, row) in rows.iter().enumerate() {
        if starts_letter(rows, ix) {
            out.push(Item::Letter(row.letter));
        }
        out.push(Item::Row(ix));
    }
    out
}

/// Whether row `ix` is the first of its letter.
fn starts_letter(rows: &[ContactRow], ix: usize) -> bool {
    ix == 0 || rows.get(ix - 1).map(|r| r.letter) != rows.get(ix).map(|r| r.letter)
}

/// The entries' sizes, shared with the virtual list.
type Sizes = Rc<Vec<gpui::Size<Pixels>>>;

/// The virtual list's entries for one read of the list, built once when
/// the rows change (not on every frame or key press), with their sizes for
/// the current text size.
pub(crate) struct Entries {
    items: Rc<Vec<Item>>,
    /// For each row, the entry to scroll to: its letter header when the
    /// row is the first of its letter (so the header comes into view with
    /// it), else the row itself.
    scroll_item: Vec<usize>,
    /// The entries that are letter headers, in order.
    headers: Vec<usize>,
    /// The sizes, and the row and header heights they were built for.
    sizes: Option<(f32, f32, Sizes)>,
    /// Each entry's top in the list, for the sizes above.
    tops: Vec<f32>,
}

/// The sticky letter header at a scroll position: the letter of the rows at
/// the top of the list, and how far it is pushed up by the next letter's
/// header as that one comes up under it (zero or less).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sticky {
    pub(crate) letter: char,
    pub(crate) shift: f32,
}

impl Entries {
    pub(crate) fn new(rows: &[ContactRow]) -> Self {
        let items = items(rows);
        let mut scroll_item = Vec::with_capacity(rows.len());
        for (at, item) in items.iter().enumerate() {
            if let Item::Row(ix) = *item {
                let header_first = at > 0 && starts_letter(rows, ix);
                scroll_item.push(if header_first { at - 1 } else { at });
            }
        }
        let headers = items
            .iter()
            .enumerate()
            .filter_map(|(at, item)| matches!(item, Item::Letter(_)).then_some(at))
            .collect();
        Self {
            items: Rc::new(items),
            scroll_item,
            headers,
            sizes: None,
            tops: Vec::new(),
        }
    }

    /// The sticky header when the list is scrolled down by `scrolled`
    /// pixels: none at the top (the first header is in place), else the
    /// letter of the entry at the top edge, pushed up by the next header
    /// once it is closer than a header's height.
    pub(crate) fn sticky(&mut self, row_h: f32, letter_h: f32, scrolled: f32) -> Option<Sticky> {
        if scrolled <= 0. || self.items.is_empty() {
            return None;
        }
        self.sizes(row_h, letter_h);
        let at = self
            .tops
            .partition_point(|&top| top <= scrolled)
            .saturating_sub(1);
        let h = self.headers.partition_point(|&i| i <= at);
        let Item::Letter(letter) = *self.items.get(*self.headers.get(h.checked_sub(1)?)?)? else {
            return None;
        };
        let shift = self
            .headers
            .get(h)
            .and_then(|&next| self.tops.get(next))
            .map_or(0., |&top| (top - scrolled - letter_h).min(0.));
        Some(Sticky { letter, shift })
    }

    /// How far down (in pixels) the list scrolls to show row `ix` whole,
    /// from `scrolled` in a viewport `view_h` high, moving the least (as
    /// `ScrollStrategy::Nearest`). A row under its own letter's header
    /// comes with the header; any other row stops a header's height below
    /// the top edge, clear of the sticky header (WCAG 2.4.11).
    pub(crate) fn scroll_for_row(
        &mut self,
        row_h: f32,
        letter_h: f32,
        ix: usize,
        scrolled: f32,
        view_h: f32,
    ) -> f32 {
        self.sizes(row_h, letter_h);
        let at = self.item_of(ix);
        let Some(&top) = self.tops.get(at) else {
            return scrolled;
        };
        let own_header = matches!(self.items.get(at), Some(Item::Letter(_)));
        let (inset, bottom) = if own_header {
            (0., top + letter_h + row_h)
        } else {
            (letter_h, top + row_h)
        };
        let to = if top - inset < scrolled {
            top - inset
        } else if bottom > scrolled + view_h {
            bottom - view_h
        } else {
            scrolled
        };
        self.clamp(row_h, letter_h, to, view_h)
    }

    /// `scrolled` kept inside the list for a viewport `view_h` high, as the
    /// virtual list keeps it when it lays out.
    pub(crate) fn clamp(&mut self, row_h: f32, letter_h: f32, scrolled: f32, view_h: f32) -> f32 {
        self.sizes(row_h, letter_h);
        let content = self.tops.last().map_or(0., |&top| {
            top + match self.items.last() {
                Some(Item::Letter(_)) => letter_h,
                _ => row_h,
            }
        });
        scrolled.min(content - view_h).max(0.)
    }

    #[cfg(test)]
    pub(crate) fn items(&self) -> &[Item] {
        &self.items
    }

    /// The entry to scroll to for row `ix` (the last entry past the end).
    pub(crate) fn item_of(&self, ix: usize) -> usize {
        self.scroll_item
            .get(ix)
            .copied()
            .unwrap_or_else(|| self.items.len().saturating_sub(1))
    }

    /// The entries' sizes for these heights (built again only when the
    /// text size changed them).
    fn sizes(&mut self, row_h: f32, letter_h: f32) -> Sizes {
        if let Some((r, l, sizes)) = &self.sizes
            && *r == row_h
            && *l == letter_h
        {
            return sizes.clone();
        }
        let sizes = Rc::new(
            self.items
                .iter()
                .map(|e| {
                    let h = match e {
                        Item::Letter(_) => letter_h,
                        Item::Row(_) => row_h,
                    };
                    size(Pixels::ZERO, px(h))
                })
                .collect::<Vec<_>>(),
        );
        let mut top = 0.;
        self.tops = sizes
            .iter()
            .map(|s| {
                let at = top;
                top += f32::from(s.height);
                at
            })
            .collect();
        self.sizes = Some((row_h, letter_h, sizes.clone()));
        sizes
    }
}

/// A contact row: the name (body) over the address (caption). 56 px at
/// 100 percent (the mockup's `.crow`), growing by what both lines grow with
/// Windows' text size, so neither clips at 150 or 225 percent.
pub(crate) fn row_h(t: &Tokens) -> f32 {
    let grown = |line: f32, base: f32| (line - base).max(0.);
    layout::CONTACT_ROW_H
        + grown(t.text.body.line.as_f32(), crate::theme::text::BODY.1)
        + grown(t.text.caption.line.as_f32(), crate::theme::text::CAPTION.1)
}

/// A letter header: one caption line, 14 px above and 4 below (the
/// mockup's `.letter`).
pub(crate) fn letter_h(t: &Tokens) -> f32 {
    space::SP_3_5 + space::SP_1 + t.text.caption.line.as_f32()
}

/// The rows: a virtual list with the letter headers, one tab stop.
fn rows(
    view: &mut ContactsView,
    window: &mut Window,
    cx: &mut Context<ContactsView>,
) -> AnyElement {
    let t = cx.tokens().clone();
    let count = view.page.rows.len();
    let (row_h, letter_h) = (row_h(&t), letter_h(&t));
    let scroll = view.list_scroll.clone();
    // The list's offset is negative once it scrolls down. The virtual list
    // only keeps it inside the list when it lays out, after this render
    // (a search that shrinks the list), so it is kept inside here too, for
    // the viewport of the last frame.
    let seen = scroll.offset();
    let view_h = f32::from(scroll.bounds().size.height);
    let built = view.entries();
    let entries = built.items.clone();
    let sizes = built.sizes(row_h, letter_h);
    let mut scrolled = -f32::from(seen.y);
    if view_h > 0. {
        scrolled = built.clamp(row_h, letter_h, scrolled, view_h);
    }
    let sticky = built.sticky(row_h, letter_h, scrolled);
    let keyboard = view.list_focus.is_focused(window) && window.last_input_was_keyboard();
    let list = v_virtual_list(
        cx.entity(),
        "contact-rows",
        sizes,
        move |this: &mut ContactsView, range: Range<usize>, _window, cx| {
            let t = cx.tokens().clone();
            let current = this.page.current();
            range
                .filter_map(|at_item| match *entries.get(at_item)? {
                    Item::Letter(l) => Some(letter(l, at_item, letter_h, &t)),
                    Item::Row(ix) => {
                        let row = this.page.rows.get(ix)?;
                        let at = current == Some(ix);
                        let open = cx.listener(move |this, _, window, cx| {
                            this.select(Some(ix), cx);
                            this.list_focus.focus(window, cx);
                        });
                        let look = RowLook {
                            ix,
                            count,
                            at,
                            ring: keyboard && at,
                            height: row_h,
                        };
                        Some(contact_row(row, look, &t, open))
                    }
                })
                .collect::<Vec<_>>()
        },
    )
    .track_scroll(&scroll)
    .size_full();
    div()
        .relative()
        .flex_1()
        .min_h_0()
        // The sticky header, pushed up, stays inside the list.
        .overflow_hidden()
        .child(
            div()
                .id("contacts-rows")
                .role(Role::List)
                .aria_label(text::contacts())
                .tab_index(0)
                .size_full()
                .on_key_down(cx.listener(ContactsView::list_key))
                .child(list)
                .test_support()
                .track_focus(&view.list_focus),
        )
        // Should the list still move after this render (a deferred scroll,
        // a new viewport), render again once, so the sticky header follows.
        .child({
            let scroll = scroll.clone();
            canvas(
                move |_, window, _| {
                    if scroll.offset() != seen {
                        window.request_animation_frame();
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_0()
        })
        .children(sticky.map(|s| sticky_letter(s, letter_h, &t)))
        .vertical_scrollbar(&scroll)
        .into_any_element()
}

/// A letter header's look: one caption line in ink-3 at the bottom of its
/// height, indented as the rows' names.
fn letter_look(l: char, height: f32, t: &Tokens) -> gpui::Div {
    div()
        .h(px(height))
        .flex_none()
        .flex()
        .items_end()
        .px(px(space::SP_2 + space::SP_3_5))
        .pb(px(space::SP_1))
        .typo(&t.text.caption)
        .text_color(t.color.ink_3)
        .child(l.to_string())
}

/// A letter header (a heading, so Narrator's heading keys jump by letter).
fn letter(l: char, at_item: usize, height: f32, t: &Tokens) -> AnyElement {
    letter_look(l, height, t)
        .id(("contact-letter", at_item))
        .role(Role::Heading)
        .aria_label(l.to_string())
        .test_support()
        .into_any_element()
}

/// The sticky header over the top of the list (Fluent's sticky list
/// header): the letter of the rows at the top while its own header is
/// scrolled away, on paper so the rows pass under it, pushed up by the
/// next letter's header. Not a heading: the header in the list is the one
/// Narrator reads, so the letter is not said twice. It takes the pointer
/// (a click there does not reach the row hidden under it) but not the
/// wheel, so the list still scrolls under it.
fn sticky_letter(s: Sticky, height: f32, t: &Tokens) -> AnyElement {
    div()
        .id("contacts-sticky-letter")
        .absolute()
        .top(px(s.shift))
        .left_0()
        .right_0()
        .bg(t.color.paper)
        .block_mouse_except_scroll()
        .child(letter_look(s.letter, height, t))
        .test_support()
        .into_any_element()
}

/// How a contact row shows.
struct RowLook {
    ix: usize,
    count: usize,
    /// The contact shown.
    at: bool,
    ring: bool,
    height: f32,
}

fn contact_row(
    row: &ContactRow,
    look: RowLook,
    t: &Tokens,
    open: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let spot_name = if !row.name.is_empty() {
        row.name.as_str()
    } else {
        row.email.as_str()
    };
    div()
        .px(px(space::SP_2))
        .child(
            ListRow::new(("contact", look.ix), text::row_name(&row.name, &row.email))
                .focusable(false)
                .selected(look.at)
                .current(look.at)
                .ring(look.ring)
                .position(look.ix + 1, look.count)
                .height(look.height)
                .on_click(open)
                .child(SenderSpot::new(spot_name).on_wash(look.at))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .typo(&t.text.body)
                                .text_color(t.color.ink)
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(row.name.clone())
                                .id(("contact-name", look.ix))
                                .test_support(),
                        )
                        .when(!row.email.is_empty(), |el| {
                            el.child(
                                div()
                                    .typo(&t.text.caption)
                                    .text_color(t.color.ink_2)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(row.email.clone())
                                    .id(("contact-email", look.ix))
                                    .test_support(),
                            )
                        }),
                ),
        )
        .into_any_element()
}

impl ContactsView {
    /// Bring row `ix` into view (with its letter header when it is the
    /// first of its letter), clear of the sticky header. The offset is set
    /// here, not left to the virtual list's next layout, so the render that
    /// follows draws the sticky header for where the list now is. Before
    /// the list has been laid out once its height is unknown, and the
    /// virtual list scrolls when it lays out.
    pub(crate) fn scroll_to_row(&mut self, ix: usize, cx: &App) {
        let t = cx.tokens();
        let (row_h, letter_h) = (row_h(t), letter_h(t));
        let view_h = f32::from(self.list_scroll.bounds().size.height);
        if view_h <= 0. {
            let at = self.entries().item_of(ix);
            self.list_scroll.scroll_to_item(at, ScrollStrategy::Nearest);
            return;
        }
        let offset = self.list_scroll.offset();
        let scrolled = -f32::from(offset.y);
        let to = self
            .entries()
            .scroll_for_row(row_h, letter_h, ix, scrolled, view_h);
        self.list_scroll.set_offset(point(offset.x, px(-to)));
    }

    /// The virtual list's entries, built when the rows changed.
    pub(crate) fn entries(&mut self) -> &mut Entries {
        let rows = &self.page.rows;
        self.entries.get_or_insert_with(|| Entries::new(rows))
    }

    /// The list's keys.
    pub(crate) fn list_key(
        &mut self,
        e: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let k = &e.keystroke;
        if k.modifiers.modified() {
            return;
        }
        let total = self.page.rows.len();
        if total == 0 {
            return;
        }
        let at = self.page.current();
        let last = total - 1;
        let to = match k.key.as_str() {
            "down" => at.map_or(0, |i| (i + 1).min(last)),
            "up" => at.map_or(0, |i| i.saturating_sub(1)),
            "home" => 0,
            "end" => last,
            "pagedown" => at.map_or(0, |i| (i + PAGE_ROWS).min(last)),
            "pageup" => at.map_or(0, |i| i.saturating_sub(PAGE_ROWS)),
            "enter" => {
                if at.is_some() {
                    self.details_shown = true;
                    cx.notify();
                }
                cx.stop_propagation();
                return;
            }
            _ => return,
        };
        self.select(Some(to), cx);
        // In one-pane layout the list stays until Enter.
        if super::one_pane(window) {
            self.details_shown = false;
        }
        cx.stop_propagation();
    }

    /// The search field's hand-over keys (as the mail list's search): Down
    /// and Enter go to the list, the other list keys move the contact
    /// shown, Esc clears the search.
    fn search_key(&mut self, key: Nav, window: &mut Window, cx: &mut Context<Self>) {
        let total = self.page.rows.len();
        if key == Nav::Escape {
            self.search.update(cx, |s, cx| s.set_value("", window, cx));
            self.page.query.clear();
            self.reload(false, cx);
            return;
        }
        if total == 0 {
            return;
        }
        let at = self.page.current();
        let last = total - 1;
        let to = match key {
            Nav::Down | Nav::Enter | Nav::SecondaryEnter => {
                if at.is_none() {
                    self.select(Some(0), cx);
                    if super::one_pane(window) {
                        self.details_shown = false;
                    }
                }
                self.list_focus.focus(window, cx);
                return;
            }
            Nav::Up => at.map_or(0, |i| i.saturating_sub(1)),
            Nav::PageDown => at.map_or(0, |i| (i + PAGE_ROWS).min(last)),
            Nav::PageUp => at.map_or(0, |i| i.saturating_sub(PAGE_ROWS)),
            Nav::Home => 0,
            Nav::End => last,
            Nav::Escape => return,
        };
        self.select(Some(to), cx);
        if super::one_pane(window) {
            self.details_shown = false;
        }
    }
}
