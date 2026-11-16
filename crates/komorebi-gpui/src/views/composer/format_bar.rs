//! The format bar, shared by the composer and the signature editor so both
//! stay in step: Gmail's order (the font and size boxes; Bold, Italic,
//! Underline and the font colour; the alignment box, the numbered and
//! bulleted lists, outdent and indent; Insert picture, quote,
//! strikethrough and clear formatting), then the owner's pinned buttons (Link, the Signature menu)
//! and More. Every command is `compose::rich`'s; the bar only surfaces
//! them.
//!
//! The boxes and toggles show the format at the caret or common to the
//! selection (`Editor::format_at`). A selection that holds two fonts,
//! sizes, colours or alignments is "mixed": the box shows no value, its
//! menu checks nothing, and its accessible name says "Mixed".
//!
//! **Overflow.** Every control has a fixed width, so what fits is known
//! from the bar's width alone ([`fit`]). When the bar is too narrow, the
//! controls leave it for More in one fixed order ([`Ctl::LEAVE_ORDER`]):
//! from the right end (Clear formatting, Strikethrough, Quote, Insert
//! picture, the indents),
//! then the boxes (alignment, size, font), then the colour, the lists, and
//! Bold, Italic and Underline last. The bar never wraps and never cuts a
//! control; More lists what left it, in the bar's order, before its own
//! items.
//!
//! **Keys.** The bar is one Tab stop, the button used last (Fluent's
//! toolbar; the Slint app's `the_formatting_row_is_one_tab_stop`); Left and
//! Right move along what is on the bar (wrapping round), Home and End go to
//! its ends.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, ElementId, EntityId, FocusHandle, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, RenderOnce, Role, SharedString, Styled as _,
    Window, canvas, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use komorebi::compose::rich::Action;
use komorebi::mail::rich::edit::Format;
use komorebi::mail::rich::{Align, ColorId, FontId, ListKind, SizePt};

use super::text::words;
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, composer as geo, layout};
use crate::widgets::button::{Button, IconButton, Variant};
use crate::widgets::menu;

/// A control of the format bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ctl {
    Font,
    Size,
    Bold,
    Italic,
    Underline,
    Colour,
    Align,
    Numbers,
    Bullets,
    Outdent,
    Indent,
    Picture,
    Quote,
    Strike,
    Clear,
}

impl Ctl {
    /// The composer's bar, in Gmail's order.
    pub const ALL: [Ctl; 15] = [
        Ctl::Font,
        Ctl::Size,
        Ctl::Bold,
        Ctl::Italic,
        Ctl::Underline,
        Ctl::Colour,
        Ctl::Align,
        Ctl::Numbers,
        Ctl::Bullets,
        Ctl::Outdent,
        Ctl::Indent,
        Ctl::Picture,
        Ctl::Quote,
        Ctl::Strike,
        Ctl::Clear,
    ];

    /// A signature's bar: the same, less what a signature cannot hold (a
    /// quote). Tables are in More, as in the composer.
    pub const SIGNATURE: [Ctl; 14] = [
        Ctl::Font,
        Ctl::Size,
        Ctl::Bold,
        Ctl::Italic,
        Ctl::Underline,
        Ctl::Colour,
        Ctl::Align,
        Ctl::Numbers,
        Ctl::Bullets,
        Ctl::Outdent,
        Ctl::Indent,
        Ctl::Picture,
        Ctl::Strike,
        Ctl::Clear,
    ];

    /// The order controls leave a bar too narrow for them, first to last.
    pub const LEAVE_ORDER: [Ctl; 15] = [
        Ctl::Clear,
        Ctl::Strike,
        Ctl::Quote,
        Ctl::Picture,
        Ctl::Indent,
        Ctl::Outdent,
        Ctl::Align,
        Ctl::Size,
        Ctl::Font,
        Ctl::Colour,
        Ctl::Numbers,
        Ctl::Bullets,
        Ctl::Underline,
        Ctl::Italic,
        Ctl::Bold,
    ];

    /// Its group on the bar (a rule goes between groups).
    fn group(self) -> u8 {
        match self {
            Ctl::Font | Ctl::Size => 0,
            Ctl::Bold | Ctl::Italic | Ctl::Underline | Ctl::Colour => 1,
            Ctl::Align | Ctl::Numbers | Ctl::Bullets | Ctl::Outdent | Ctl::Indent => 2,
            Ctl::Picture | Ctl::Quote | Ctl::Strike | Ctl::Clear => 3,
        }
    }

    /// Its width on the bar.
    pub fn width(self) -> f32 {
        match self {
            Ctl::Font => geo::FONT_BOX_W,
            Ctl::Size => geo::SIZE_BOX_W,
            Ctl::Align => geo::ALIGN_BOX_W,
            _ => SLOT_W,
        }
    }

    /// The last part of its element id (`compose-bold`, `sig-bold`).
    fn slug(self) -> &'static str {
        match self {
            Ctl::Font => "font",
            Ctl::Size => "size",
            Ctl::Bold => "bold",
            Ctl::Italic => "italic",
            Ctl::Underline => "underline",
            Ctl::Colour => "colour",
            Ctl::Align => "align",
            Ctl::Numbers => "numbers",
            Ctl::Bullets => "bullets",
            Ctl::Outdent => "outdent",
            Ctl::Indent => "indent",
            Ctl::Picture => "picture",
            Ctl::Quote => "quote",
            Ctl::Strike => "strike",
            Ctl::Clear => "clear",
        }
    }

    /// A toggle's command, icon, accessible name, tooltip and state; `None`
    /// for the boxes.
    fn button(self, f: &Format) -> Option<(Action, Icon, String, String, Option<bool>)> {
        let on = |b: bool| Some(b);
        Some(match self {
            Ctl::Bold => (
                Action::Bold,
                Icon::Bold,
                words::bold(),
                words::bold(),
                on(f.b),
            ),
            Ctl::Italic => (
                Action::Italic,
                Icon::Italic,
                words::italic(),
                words::italic(),
                on(f.i),
            ),
            Ctl::Underline => (
                Action::Underline,
                Icon::Underline,
                words::underline(),
                words::underline(),
                on(f.u),
            ),
            Ctl::Strike => (
                Action::Strike,
                Icon::Strikethrough,
                words::strike(),
                words::strike(),
                on(f.s),
            ),
            Ctl::Numbers => (
                Action::Numbers,
                Icon::Numbers,
                words::numbers(),
                words::numbers(),
                on(f.list == Some(ListKind::Number)),
            ),
            Ctl::Bullets => (
                Action::Bullets,
                Icon::Bullets,
                words::bullets(),
                words::bullets(),
                on(f.list == Some(ListKind::Bullet)),
            ),
            Ctl::Outdent => (
                Action::Indent(false),
                Icon::IndentLess,
                words::outdent(),
                words::outdent_tip(),
                None,
            ),
            Ctl::Indent => (
                Action::Indent(true),
                Icon::IndentMore,
                words::indent(),
                words::indent_tip(),
                None,
            ),
            Ctl::Picture => (
                Action::InsertPicture,
                Icon::Image,
                words::insert_picture(),
                words::insert_picture(),
                None,
            ),
            Ctl::Quote => (
                Action::Quote,
                Icon::QuoteText,
                words::quote(),
                words::quote_tip(),
                on(f.quote),
            ),
            Ctl::Clear => (
                Action::Clear,
                Icon::ClearFormat,
                words::clear(),
                words::clear_tip(),
                None,
            ),
            Ctl::Font | Ctl::Size | Ctl::Colour | Ctl::Align => return None,
        })
    }
}

/// A round button's slot on the bar (the toggles, Link, Signature, More).
pub const SLOT_W: f32 = layout::ICON_BTN_SM;
/// A rule between groups, with its room.
pub const SEP_W: f32 = geo::BAR_SEP_W;

/// The bar's controls (and how many fixed slots follow them, and More)
/// side by side: their widths, the rules between groups and before the
/// slots, and the gaps between them all.
pub fn bar_width(shown: &[Ctl], slots: usize) -> f32 {
    let mut items: Vec<f32> = Vec::new();
    for (i, c) in shown.iter().enumerate() {
        if i > 0 && shown[i - 1].group() != c.group() {
            items.push(SEP_W);
        }
        items.push(c.width());
    }
    if slots > 0 {
        if !shown.is_empty() {
            items.push(SEP_W);
        }
        items.extend(std::iter::repeat_n(SLOT_W, slots));
    }
    let gaps = items.len().saturating_sub(1) as f32;
    items.iter().sum::<f32>() + geo::BAR_GAP * gaps
}

/// The controls of `ctls` that fit in `room` px beside `pinned` slots and
/// More (always there when `more_always`; else only when something left
/// the bar), in the bar's order: the least taken off in
/// [`Ctl::LEAVE_ORDER`].
pub fn fit(room: f32, ctls: &[Ctl], pinned: usize, more_always: bool) -> Vec<Ctl> {
    let leaving: Vec<Ctl> = Ctl::LEAVE_ORDER
        .iter()
        .copied()
        .filter(|c| ctls.contains(c))
        .collect();
    for gone in 0..=leaving.len() {
        let shown: Vec<Ctl> = ctls
            .iter()
            .copied()
            .filter(|c| !leaving[..gone].contains(c))
            .collect();
        let more = more_always || gone > 0;
        if bar_width(&shown, pinned + usize::from(more)) <= room {
            return shown;
        }
    }
    Vec::new()
}

/// Where a key moves the focus along `count` buttons from button `at`
/// (`None`: not a key of the bar). Left and Right wrap round.
pub fn step(key: &str, at: usize, count: usize) -> Option<usize> {
    let last = count.checked_sub(1)?;
    match key {
        "left" => Some(if at == 0 { last } else { at - 1 }),
        "right" => Some(if at >= last { 0 } else { at + 1 }),
        "home" => Some(0),
        "end" => Some(last),
        _ => None,
    }
}

/// The most buttons a bar holds: every control, two pinned, and More.
const MAX_SLOTS: usize = Ctl::ALL.len() + 3;

/// A bar's state, kept by its owner across frames: a focus handle per
/// button on the bar (in its order), the one that is the Tab stop, how
/// many show, and the bar's measured room.
#[derive(Clone)]
pub struct BarState {
    handles: Vec<FocusHandle>,
    at: Rc<Cell<usize>>,
    count: Rc<Cell<usize>>,
    room: Rc<Cell<Option<f32>>>,
}

impl BarState {
    pub fn new(cx: &mut App) -> Self {
        Self {
            handles: (0..MAX_SLOTS).map(|_| cx.focus_handle()).collect(),
            at: Rc::new(Cell::new(0)),
            count: Rc::new(Cell::new(0)),
            room: Rc::new(Cell::new(None)),
        }
    }

    /// The buttons' focus handles, in the bar's order (the ones past
    /// [`BarState::count`] are not on it).
    pub fn handles(&self) -> &[FocusHandle] {
        &self.handles
    }

    /// The button that is the bar's Tab stop.
    pub fn at(&self) -> usize {
        self.at.get()
    }

    /// How many buttons the bar showed last.
    pub fn count(&self) -> usize {
        self.count.get()
    }

    /// The room measured last (none before the first frame).
    pub fn room(&self) -> Option<f32> {
        self.room.get()
    }

    /// Button `i` as it is drawn: the bar's one Tab stop when it is the one
    /// used last.
    fn slot(&self, i: usize) -> FocusHandle {
        self.handles[i]
            .clone()
            .tab_index(0)
            .tab_stop(i == self.at.get())
    }
}

type OnAction = Rc<dyn Fn(&Action, &mut Window, &mut App)>;
type MenuFn = Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;
type Pinned = Box<dyn FnOnce(FocusHandle) -> AnyElement>;

/// The format bar.
#[derive(IntoElement)]
pub struct FormatBar {
    prefix: &'static str,
    ctls: &'static [Ctl],
    format: Format,
    state: BarState,
    owner: EntityId,
    on_action: OnAction,
    pinned: Vec<Pinned>,
    more: Option<MenuFn>,
    pad: (f32, f32),
}

impl FormatBar {
    /// A bar of `ctls` over `format` (the editor's at the caret), its
    /// element ids starting `prefix-`. `on_action` runs a command on the
    /// owner's editor; `owner` is notified when the bar's width changes.
    pub fn new(
        prefix: &'static str,
        ctls: &'static [Ctl],
        format: Format,
        state: &BarState,
        owner: EntityId,
        on_action: impl Fn(&Action, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            prefix,
            ctls,
            format,
            state: state.clone(),
            owner,
            on_action: Rc::new(on_action),
            pinned: Vec::new(),
            more: None,
            pad: (0., 0.),
        }
    }

    /// A round button after the controls that never leaves the bar (Link,
    /// the Signature menu), given its focus handle.
    pub fn pinned(mut self, button: impl FnOnce(FocusHandle) -> AnyElement + 'static) -> Self {
        self.pinned.push(Box::new(button));
        self
    }

    /// More's own items, after what left the bar; More then always shows.
    pub fn more(
        mut self,
        items: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        self.more = Some(Rc::new(items));
        self
    }

    /// The bar's padding (horizontal, vertical).
    pub fn padding(mut self, x: f32, y: f32) -> Self {
        self.pad = (x, y);
        self
    }

    fn id(&self, c: Ctl) -> ElementId {
        ElementId::Name(SharedString::from(format!("{}-{}", self.prefix, c.slug())))
    }

    /// Control `c` with focus handle `focus`.
    fn control(&self, c: Ctl, focus: &FocusHandle, slot: usize) -> AnyElement {
        let f = &self.format;
        let on = self.on_action.clone();
        let at = self.state.at.clone();
        if let Some((action, icon, name, tip, state)) = c.button(f) {
            return IconButton::new(self.id(c), icon, name)
                .small()
                .tooltip(tip)
                .when_some(state, |b, on| b.toggled(on))
                .track_focus(focus)
                .on_click(move |_, window, cx| {
                    at.set(slot);
                    on(&action, window, cx)
                })
                .into_any_element();
        }
        let mixed = f.mixed;
        let value = |shown: String, is_mixed: bool| {
            if is_mixed { words::mixed() } else { shown }
        };
        match c {
            Ctl::Font => {
                let label = (!mixed.font).then(|| f.font.label().to_string());
                let name = words::valued(
                    &words::font(),
                    &value(f.font.label().to_string(), mixed.font),
                );
                let (font, checked) = (f.font, !mixed.font);
                box_button(
                    self.id(c),
                    geo::FONT_BOX_W,
                    label,
                    None,
                    name,
                    words::font(),
                )
                .track_focus(focus)
                .dropdown_menu(move |m, _, cx| font_menu(menu::brand(m, cx), font, checked, &on))
                .into_any_element()
            }
            Ctl::Size => {
                let pt = f.size.get().to_string();
                let label = (!mixed.size).then(|| pt.clone());
                let name = words::valued(&words::font_size(), &value(pt, mixed.size));
                let (size, checked) = (f.size, !mixed.size);
                box_button(
                    self.id(c),
                    geo::SIZE_BOX_W,
                    label,
                    None,
                    name,
                    words::font_size_tip(),
                )
                .track_focus(focus)
                .dropdown_menu(move |m, _, cx| size_menu(menu::brand(m, cx), size, checked, &on))
                .into_any_element()
            }
            Ctl::Align => {
                let name = words::valued(
                    &words::alignment(),
                    &value(align_label(f.align), mixed.align),
                );
                let icon = if mixed.align {
                    Icon::AlignLeft
                } else {
                    align_icon(f.align)
                };
                let (align, checked) = (f.align, !mixed.align);
                box_button(
                    self.id(c),
                    geo::ALIGN_BOX_W,
                    None,
                    Some(icon),
                    name,
                    words::alignment(),
                )
                .track_focus(focus)
                .dropdown_menu(move |m, _, cx| align_menu(menu::brand(m, cx), align, checked, &on))
                .into_any_element()
            }
            _ => {
                let name = words::valued(
                    &words::font_colour(),
                    &value(words::colour(f.color), mixed.color),
                );
                let (colour, checked) = (f.color, !mixed.color);
                IconButton::new(self.id(c), Icon::TextColor, name)
                    .small()
                    .tooltip(words::font_colour())
                    .track_focus(focus)
                    .dropdown_menu(move |m, _, cx| {
                        colour_menu(menu::brand(m, cx), colour, checked, &on)
                    })
                    .into_any_element()
            }
        }
    }
}

/// A box of the bar: a compact subtle pill of a fixed width with its value
/// (none when mixed) or glyph, and a chevron.
fn box_button(
    id: ElementId,
    width: f32,
    label: Option<String>,
    icon: Option<Icon>,
    name: String,
    tip: String,
) -> Button {
    let mut b = Button::new(id)
        .compact()
        .variant(Variant::Subtle)
        .label(label.unwrap_or_default())
        .name(name)
        .tooltip(tip)
        .expandable()
        .trailing(Icon::ChevronDown);
    if let Some(icon) = icon {
        b = b.icon(icon);
    }
    b.w(px(width))
}

fn align_icon(a: Align) -> Icon {
    match a {
        Align::Left => Icon::AlignLeft,
        Align::Center => Icon::AlignCenter,
        Align::Right => Icon::AlignRight,
        Align::Justify => Icon::AlignJustify,
    }
}

fn align_label(a: Align) -> String {
    match a {
        Align::Left => words::align_left(),
        Align::Center => words::align_center(),
        Align::Right => words::align_right(),
        Align::Justify => words::justify(),
    }
}

/// A menu item that runs `action`.
fn pick(label: String, checked: bool, action: Action, on: &OnAction) -> PopupMenuItem {
    let on = on.clone();
    PopupMenuItem::new(label)
        .checked(checked)
        .on_click(move |_, window, cx| on(&action, window, cx))
}

/// The font box's menu: the ten fonts, the current one checked (none when
/// the selection is mixed).
fn font_menu(m: PopupMenu, font: FontId, checked: bool, on: &OnAction) -> PopupMenu {
    FontId::ALL.iter().fold(m, |m, id| {
        m.item(pick(
            id.label().to_string(),
            checked && *id == font,
            Action::Font(*id),
            on,
        ))
    })
}

fn size_menu(m: PopupMenu, size: SizePt, checked: bool, on: &OnAction) -> PopupMenu {
    SizePt::ALL
        .iter()
        .filter_map(|pt| SizePt::new(*pt))
        .fold(m, |m, pt| {
            m.item(pick(
                pt.get().to_string(),
                checked && pt == size,
                Action::Size(pt),
                on,
            ))
        })
}

fn colour_menu(m: PopupMenu, colour: Option<ColorId>, checked: bool, on: &OnAction) -> PopupMenu {
    // A pasted colour outside the named ten: listed last, checked.
    let custom = colour.filter(|c| matches!(c, ColorId::Rgb(..)));
    std::iter::once(None)
        .chain(ColorId::ALL.iter().copied().map(Some))
        .chain(custom.map(Some))
        .fold(m, |m, c| {
            m.item(pick(
                words::colour(c),
                checked && c == colour,
                Action::Color(c),
                on,
            ))
        })
}

fn align_menu(m: PopupMenu, align: Align, checked: bool, on: &OnAction) -> PopupMenu {
    [Align::Left, Align::Center, Align::Right, Align::Justify]
        .into_iter()
        .fold(m, |m, a| {
            m.item(pick(
                align_label(a),
                checked && a == align,
                Action::Align(a),
                on,
            ))
        })
}

/// The controls that left the bar, at the head of More: the toggles as
/// checked items, the boxes as submenus.
fn overflow_items(
    mut m: PopupMenu,
    over: &[Ctl],
    f: &Format,
    on: &OnAction,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let mixed = f.mixed;
    for c in over {
        if let Some((action, _, _, _, state)) = c.button(f) {
            let label = match c {
                Ctl::Bold => words::bold(),
                Ctl::Italic => words::italic(),
                Ctl::Underline => words::underline(),
                Ctl::Strike => words::strike(),
                Ctl::Numbers => words::numbers(),
                Ctl::Bullets => words::bullets(),
                Ctl::Outdent => words::outdent(),
                Ctl::Indent => words::indent(),
                Ctl::Picture => words::insert_picture(),
                Ctl::Quote => words::quote(),
                _ => words::clear(),
            };
            m = m.item(pick(label, state.unwrap_or(false), action, on));
            continue;
        }
        let on = on.clone();
        m = match c {
            Ctl::Font => {
                let (font, checked) = (f.font, !mixed.font);
                m.submenu(words::font(), window, cx, move |m, _, cx| {
                    font_menu(menu::brand(m, cx), font, checked, &on)
                })
            }
            Ctl::Size => {
                let (size, checked) = (f.size, !mixed.size);
                m.submenu(words::font_size(), window, cx, move |m, _, cx| {
                    size_menu(menu::brand(m, cx), size, checked, &on)
                })
            }
            Ctl::Align => {
                let (align, checked) = (f.align, !mixed.align);
                m.submenu(words::alignment(), window, cx, move |m, _, cx| {
                    align_menu(menu::brand(m, cx), align, checked, &on)
                })
            }
            _ => {
                let (colour, checked) = (f.color, !mixed.color);
                m.submenu(words::font_colour(), window, cx, move |m, _, cx| {
                    colour_menu(menu::brand(m, cx), colour, checked, &on)
                })
            }
        };
    }
    m
}

impl RenderOnce for FormatBar {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let more_always = self.more.is_some();
        let pinned = std::mem::take(&mut self.pinned);
        // Before the first measure, everything; the measure then decides.
        let shown: Vec<Ctl> = match self.state.room.get() {
            Some(room) => fit(room, self.ctls, pinned.len(), more_always),
            None => self.ctls.to_vec(),
        };
        let over: Vec<Ctl> = self
            .ctls
            .iter()
            .copied()
            .filter(|c| !shown.contains(c))
            .collect();
        let has_more = more_always || !over.is_empty();
        let count = shown.len() + pinned.len() + usize::from(has_more);
        // The Tab stop: the button focused now, else the one used last.
        let st = self.state.clone();
        st.count.set(count);
        if let Some(i) = st.handles[..count]
            .iter()
            .position(|h| h.is_focused(window))
        {
            st.at.set(i);
        }
        if st.at.get() >= count {
            st.at.set(count.saturating_sub(1));
        }
        let rule = || {
            div()
                .flex_none()
                .w(px(layout::HAIRLINE))
                .h(px(geo::BAR_SEP_H))
                .mx(px(geo::BAR_SEP_SPACE))
                .bg(t.color.line)
                .into_any_element()
        };
        let mut children: Vec<AnyElement> = Vec::new();
        for (i, c) in shown.iter().enumerate() {
            if i > 0 && shown[i - 1].group() != c.group() {
                children.push(rule());
            }
            children.push(self.control(*c, &st.slot(i), i));
        }
        let mut slot = shown.len();
        if (!pinned.is_empty() || has_more) && !shown.is_empty() {
            children.push(rule());
        }
        for p in pinned {
            children.push(p(st.slot(slot)));
            slot += 1;
        }
        if has_more {
            let (format, on, extra) = (self.format.clone(), self.on_action.clone(), self.more);
            children.push(
                IconButton::new(
                    ElementId::Name(SharedString::from(format!("{}-more", self.prefix))),
                    Icon::More,
                    words::more(),
                )
                .small()
                .track_focus(&st.slot(slot))
                .dropdown_menu(move |m, window, cx| {
                    let mut m = overflow_items(menu::brand(m, cx), &over, &format, &on, window, cx);
                    if let Some(extra) = &extra {
                        m = extra(m.separator(), window, cx);
                    }
                    m
                })
                .into_any_element(),
            );
        }
        let owner = self.owner;
        let pad_x = self.pad.0;
        let measure = {
            let room = st.room.clone();
            canvas(
                move |bounds, window, cx| {
                    let w = f32::from(bounds.size.width) - pad_x * 2.;
                    if room.get().is_none_or(|r| (r - w).abs() > 0.5) {
                        room.set(Some(w));
                        window.defer(cx, move |_, cx| cx.notify(owner));
                    }
                },
                |_, (), _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full()
        };
        let keys = st.clone();
        div()
            .id(ElementId::Name(SharedString::from(format!(
                "{}-format-bar",
                self.prefix
            ))))
            .role(Role::Toolbar)
            .aria_label(SharedString::from(words::formatting()))
            .relative()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(geo::BAR_GAP))
            .px(px(self.pad.0))
            .py(px(self.pad.1))
            .on_key_down(move |e: &KeyDownEvent, window, cx| {
                let k = &e.keystroke;
                if k.modifiers.modified() {
                    return;
                }
                let count = keys.count.get();
                let Some(at) = keys.handles[..count]
                    .iter()
                    .position(|h| h.is_focused(window))
                else {
                    return;
                };
                let Some(to) = step(&k.key, at, count) else {
                    return;
                };
                keys.at.set(to);
                window.focus(&keys.handles[to], cx);
                cx.notify(owner);
                cx.stop_propagation();
            })
            .child(measure)
            .children(children)
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything fits a wide bar; More is there only when asked for or
    /// when something left.
    #[test]
    fn a_wide_bar_holds_every_control() {
        let all = bar_width(&Ctl::ALL, 3);
        assert_eq!(fit(all, &Ctl::ALL, 2, true), Ctl::ALL.to_vec());
        assert_eq!(fit(all + 100., &Ctl::ALL, 2, true), Ctl::ALL.to_vec());
        // The signature's bar has no More of its own until it overflows.
        let sig = bar_width(&Ctl::SIGNATURE, 1);
        assert_eq!(fit(sig, &Ctl::SIGNATURE, 1, false), Ctl::SIGNATURE.to_vec());
    }

    /// Port of the Slint toolbar's overflow: as the bar narrows, controls
    /// leave in the fixed order, one at a time (the boxes before the
    /// toggles, Bold last), and what stays keeps the bar's order.
    #[test]
    fn controls_leave_a_narrow_bar_in_the_fixed_order() {
        let mut left: Vec<Ctl> = Vec::new();
        let mut before = Ctl::ALL.to_vec();
        let mut room = bar_width(&Ctl::ALL, 3);
        while room > 0. {
            room -= 1.;
            let now = fit(room, &Ctl::ALL, 2, true);
            // Nothing but Link, Signature and More: they never leave.
            assert!(
                now.is_empty() || bar_width(&now, 3) <= room,
                "fits: {now:?} in {room}"
            );
            // What stays is in the bar's order.
            let order: Vec<Ctl> = Ctl::ALL
                .iter()
                .copied()
                .filter(|c| now.contains(c))
                .collect();
            assert_eq!(now, order);
            for c in before.iter().filter(|c| !now.contains(c)) {
                left.push(*c);
            }
            assert!(
                now.iter().all(|c| before.contains(c)),
                "nothing comes back as the bar narrows"
            );
            before = now;
        }
        assert_eq!(left, Ctl::LEAVE_ORDER.to_vec());
        assert!(fit(0., &Ctl::ALL, 2, true).is_empty());
    }

    /// At the composer's narrowest (a 720 px window: rail, list and the
    /// reading pane, about 320 px for the composer) Bold, Italic and
    /// Underline stay on the bar beside Link, Signature and More.
    #[test]
    fn the_narrowest_composer_keeps_bold_italic_underline() {
        let room = 320. - geo::BAR_PAD_X * 2.;
        let shown = fit(room, &Ctl::ALL, 2, true);
        for c in [Ctl::Bold, Ctl::Italic, Ctl::Underline] {
            assert!(shown.contains(&c), "{shown:?}");
        }
        assert!(!shown.contains(&Ctl::Clear));
    }

    /// The rules between groups are counted: four groups, then the rule
    /// before the slots.
    #[test]
    fn the_width_counts_controls_rules_and_gaps() {
        let w = bar_width(&[Ctl::Font, Ctl::Bold], 1);
        let want = geo::FONT_BOX_W + SEP_W + SLOT_W + SEP_W + SLOT_W + geo::BAR_GAP * 4.;
        assert!((w - want).abs() < 0.01, "{w} {want}");
        assert_eq!(bar_width(&[], 0), 0.);
        assert_eq!(bar_width(&[], 2), SLOT_W * 2. + geo::BAR_GAP);
    }

    #[test]
    fn keys_move_along_what_is_on_the_bar() {
        assert_eq!(step("left", 0, 6), Some(5));
        assert_eq!(step("right", 5, 6), Some(0));
        assert_eq!(step("right", 2, 6), Some(3));
        assert_eq!(step("end", 2, 6), Some(5));
        assert_eq!(step("home", 4, 6), Some(0));
        assert_eq!(step("up", 1, 6), None);
        assert_eq!(step("left", 0, 0), None, "an empty bar");
    }
}
