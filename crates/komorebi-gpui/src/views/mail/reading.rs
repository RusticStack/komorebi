//! The reading pane around the reader and composer slots (mockups
//! `inbox`, `multi`): with a message open, the command bar (Reply, Reply all
//! and Forward; Archive, Delete and Move; Mark as read or unread and Mark as
//! important; More at the end, as the Slint bar's groups; labels by
//! priority as the width allows, as the Slint bar) over the reader; with several messages
//! selected, what they are and their actions (Archive, Mark as read,
//! Delete, Select all, Clear selection); the composer while it is open.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, Context, FocusHandle, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Role, SharedString, Styled, TextRun, Window, canvas, div,
    prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::Selectable as _;
use gpui_kit::component::menu::DropdownMenu as _;

use super::model::Mail;
use super::text::{self, Command};
use super::triage::Flag;
use super::{MailView, commands, menus};
use crate::access::Key;
use crate::icons::Icon;
use crate::theme::metrics::scaled;
use crate::theme::{ActiveTokens as _, TextStyled as _, Tokens, layout, space};
use crate::views::composer::Draft;
use crate::widgets::button::{Button, Height, IconButton, Variant};
use crate::widgets::pane::Pane;
use komorebi_i18n::tr;

/// A command bar button's tooltip, with its key (the Slint bar's).
fn tip(cmd: Command) -> String {
    match cmd {
        Command::Reply => tr!("Reply (Ctrl+R)"),
        Command::ReplyAll => tr!("Reply all (Ctrl+Shift+R)"),
        Command::Forward => tr!("Forward (Ctrl+Shift+F)"),
        Command::Flag => tr!("Mark as important (Ctrl+Shift+G)"),
        Command::Unflag => tr!("Remove importance (Ctrl+Shift+G)"),
        other => text::command_tip(other),
    }
}

pub fn render(
    view: &mut MailView,
    composing: bool,
    one_pane: bool,
    window: &mut Window,
    cx: &mut Context<MailView>,
) -> AnyElement {
    let t = cx.tokens().clone();
    let content: AnyElement = if composing {
        // A cached view: a key typed in the body redraws the composer alone
        // (the list, the folders and the window around it are replayed).
        view.composer
            .clone()
            .cached(gpui::StyleRefinement::default().size_full())
            .into_any_element()
    } else if view.mail.open_id().is_some() {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(command_bar(view, one_pane, &t, window, cx))
            .child(div().flex_1().min_h_0().child(view.reader.clone()))
            .into_any_element()
    } else if view.mail.selection().len() > 1 {
        selection(view, &t, cx)
    } else if let Some(entry) = super::outbox::render(view, &t, cx) {
        entry
    } else {
        div().into_any_element()
    };
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        // The message commands' keys reach the open message from here too
        // (never from the composer, whose fields take their own keys).
        .when(!composing, |el| el.key_context(commands::MESSAGES))
        .child(
            Pane::new("reading-pane").child(
                div()
                    .id("reading")
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(div().flex_1().min_h_0().child(content))
                    // One pane: the list is hidden, so its status line and Undo
                    // offer show at the message's foot (the Slint app's).
                    .when(one_pane, |el| {
                        el.children(super::status::line(&view.hub, cx))
                            .children(super::list::status(view, cx))
                            .children(super::list::undo(view, cx))
                    })
                    .track_focus(&view.reading_focus),
            ),
        )
        .into_any_element()
}

/// The command bar's buttons in their order, for its one Tab stop.
const BAR_IDS: &[&str] = &[
    "cmd-back",
    "cmd-reply",
    "cmd-reply-all",
    "cmd-forward",
    "cmd-archive",
    "cmd-delete",
    "cmd-move",
    "cmd-read",
    "cmd-flag",
    "cmd-more",
];

/// Where Left, Right, Home and End move along a toolbar of `count` buttons
/// from `at` (Fluent and UI Automation toolbars: one Tab stop, the arrows
/// go along it and wrap). `None`: not one of those keys.
pub fn bar_step(key: &str, at: usize, count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let at = at.min(count - 1);
    match key {
        "left" => Some(if at == 0 { count - 1 } else { at - 1 }),
        "right" => Some(if at + 1 == count { 0 } else { at + 1 }),
        "home" => Some(0),
        "end" => Some(count - 1),
        _ => None,
    }
}

#[cfg(test)]
thread_local! {
    /// The last bar's buttons and their focus handles (tests: Kit's
    /// queries see no focus handle a widget tracks inside its render).
    static BAR_FOCUS: RefCell<Vec<(&'static str, FocusHandle)>> = const { RefCell::new(Vec::new()) };
    /// The buttons the last bar labelled.
    static BAR_LABELS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

/// The labels kept first, strictly in this order: a later one only once
/// those before it have theirs. Reply, the reader's primary action, is the
/// last label to go; Archive and Delete become icons before it.
const STRICT: &[&str] = &["cmd-reply", "cmd-archive", "cmd-delete"];
/// Then each of these while it still fits after the ones before it.
const THEN: &[&str] = &[
    "cmd-read",
    "cmd-flag",
    "cmd-move",
    "cmd-reply-all",
    "cmd-forward",
];

/// The buttons that get their label (row 160, the Slint bar's labels by
/// priority, each measured on its own): `extra` is what each shown button
/// adds to the bar with its label, `room` what the bar has over its width
/// with no label. Reply, Archive and Delete strictly in that order (one not
/// shown is skipped: Reply on a server draft); then Mark as read, Mark as
/// important, Move, Reply all and Forward, each while it fits, once those
/// three have theirs.
fn labels_by_priority(mut room: f32, extra: &[(&'static str, f32)]) -> Vec<&'static str> {
    let of = |id: &str| extra.iter().find(|(i, _)| *i == id).map(|(_, w)| *w);
    let mut on = Vec::new();
    for id in STRICT {
        let Some(w) = of(id) else { continue };
        if w > room + 0.5 {
            return on;
        }
        room -= w;
        on.push(*id);
    }
    if on.is_empty() {
        return on;
    }
    for id in THEN {
        if let Some(w) = of(id)
            && w <= room + 0.5
        {
            room -= w;
            on.push(*id);
        }
    }
    on
}

/// How a bar button looks with no label, for what its label adds.
#[derive(Clone, Copy)]
enum Plain {
    /// A round icon button.
    Icon,
    /// A pill with its icon only (Reply).
    Pill,
    /// A pill with its icon and a trailing chevron (Move).
    Menu,
}

/// What `label` adds to a bar button that is `plain` without it: the
/// text, the gap before it and, for an icon button, the pill's padding
/// over the round button (`widgets::button`).
fn label_extra(text_w: f32, plain: Plain, t: &Tokens) -> f32 {
    let h = Height::Medium;
    let pill = h.icon_pad() + h.icon() + space::SP_4;
    match plain {
        Plain::Icon => pill + space::SP_2 + text_w - layout::ICON_BTN,
        Plain::Pill => {
            let without = pill.max(scaled::control(t, layout::CONTROL_H_MD));
            pill + space::SP_2 + text_w - without
        }
        Plain::Menu => space::SP_2 + text_w,
    }
}

/// The width of `text` in the bar's label type (a pill's body text).
fn text_width(text: &SharedString, t: &Tokens, window: &mut Window) -> f32 {
    let mut style = window.text_style();
    style.font_family = t.text.body.family.clone();
    style.font_weight = t.text.body.weight;
    let run = TextRun {
        len: text.len(),
        font: style.font(),
        color: t.color.ink,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(text.clone(), t.text.body.size, &[run], None);
    f32::from(line.width)
}

/// A command bar button: an icon button, or with its label a subtle pill.
struct Cmd {
    id: &'static str,
    glyph: Icon,
    name: SharedString,
    tip: String,
    toggled: Option<bool>,
    key: Option<Key>,
}

impl Cmd {
    fn new(id: &'static str, glyph: Icon, cmd: Command) -> Self {
        Cmd {
            id,
            glyph,
            name: text::command(cmd).into(),
            tip: tip(cmd),
            toggled: None,
            key: None,
        }
    }

    fn render(
        self,
        rove: &Roving,
        labelled: bool,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        if labelled {
            let on = self.toggled == Some(true);
            let b = Button::new(self.id)
                .icon(self.glyph)
                .label(self.name)
                .variant(Variant::Subtle)
                .height(Height::Medium)
                .tooltip(self.tip)
                .selected(on)
                .when_some(self.toggled, |b, on| {
                    b.aria_toggled(if on {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                })
                .on_click(on_click);
            let b = match self.key {
                Some(key) => b.access_key(key),
                None => b,
            };
            rove.track(self.id, b).into_any_element()
        } else {
            let b = IconButton::new(self.id, self.glyph, self.name)
                .tooltip(self.tip)
                .when_some(self.toggled, |b, on| b.toggled(on))
                .on_click(on_click);
            let b = match self.key {
                Some(key) => b.access_key(key),
                None => b,
            };
            rove.track(self.id, b).into_any_element()
        }
    }
}

/// The command bar's roving Tab stop: a focus handle per button that shows
/// (kept between frames under the button's id), of which one is a Tab stop:
/// the button last focused, else the first. Left and Right go along.
struct Roving {
    ids: Vec<&'static str>,
    handles: Vec<FocusHandle>,
}

impl Roving {
    fn new(ids: Vec<&'static str>, window: &mut Window, cx: &mut App) -> Self {
        let stop = window.use_keyed_state("command-bar-stop", cx, |_, _| None::<&'static str>);
        let raw: Vec<FocusHandle> = ids
            .iter()
            .map(|id| {
                window
                    .use_keyed_state(SharedString::from(format!("{id}-focus")), cx, |_, cx| {
                        cx.focus_handle()
                    })
                    .read(cx)
                    .clone()
            })
            .collect();
        // A click or a key put the focus on a button: it is the stop now, so
        // Shift+Tab from it leaves the bar instead of going back along it.
        if let Some(i) = raw.iter().position(|h| h.is_focused(window)) {
            stop.update(cx, |s, _| *s = Some(ids[i]));
        }
        let at = stop
            .read(cx)
            .and_then(|s| ids.iter().position(|id| *id == s))
            .unwrap_or(0);
        let handles = raw
            .into_iter()
            .enumerate()
            .map(|(i, h)| h.tab_index(0).tab_stop(i == at))
            .collect();
        let rove = Roving { ids, handles };
        #[cfg(test)]
        BAR_FOCUS.with(|b| {
            *b.borrow_mut() = rove.ids.iter().copied().zip(rove.handles.clone()).collect()
        });
        rove
    }

    /// `el` takes the bar's focus handle for `id` (a button that shows and
    /// is enabled).
    fn track<E: InteractiveElement>(&self, id: &str, el: E) -> E {
        match self.ids.iter().position(|i| *i == id) {
            Some(i) => el.track_focus(&self.handles[i]),
            None => el,
        }
    }
}

/// What the bar measured last frame: its width, the width it needs with
/// no label, where its buttons start, and where each button that can go
/// to More ends.
#[derive(Default)]
struct Fit {
    bar: Cell<Option<f32>>,
    /// The bar's width with every button an icon (Reply and Move their
    /// unlabelled pills).
    plain: Cell<Option<f32>>,
    /// The buttons' group's left edge (window px).
    group_left: Cell<Option<f32>>,
    ends: RefCell<Ends>,
}

/// Each button's right end from the group's start, with no label, for the
/// bar's buttons `set` (another set, such as a server draft's, places them
/// elsewhere).
#[derive(Default)]
struct Ends {
    set: Vec<&'static str>,
    at: HashMap<&'static str, f32>,
}

/// The buttons that can go to More when the bar is too narrow for them.
const OVERFLOWS: &[&str] = &[
    "cmd-reply-all",
    "cmd-forward",
    "cmd-archive",
    "cmd-delete",
    "cmd-move",
    "cmd-read",
    "cmd-flag",
];

/// The buttons of `shown` (in the bar's order) that go to More: from the
/// first that can and whose end (`ends`) passes `room` to the last. Back
/// and Reply stay; a button not measured yet fits until it is.
fn overflowed(
    shown: &[&'static str],
    ends: &HashMap<&'static str, f32>,
    room: f32,
) -> Vec<&'static str> {
    let first = shown
        .iter()
        .position(|id| OVERFLOWS.contains(id) && ends.get(id).is_some_and(|&end| end > room + 0.5));
    first.map_or_else(Vec::new, |i| {
        shown[i..]
            .iter()
            .copied()
            .filter(|id| OVERFLOWS.contains(id))
            .collect()
    })
}

impl Fit {
    /// The buttons of `shown` that do not fit the bar with no label: they
    /// leave the bar (and its arrow order) for the start of More, as a
    /// Fluent toolbar's overflow.
    fn overflow(&self, shown: &[&'static str]) -> Vec<&'static str> {
        let mut ends = self.ends.borrow_mut();
        if ends.set != shown {
            *ends = Ends {
                set: shown.to_vec(),
                at: HashMap::new(),
            };
        }
        let Some(bar) = self.bar.get() else {
            return Vec::new();
        };
        // The bar's sides, the gaps around its spacer and More.
        let room = bar - space::SP_3 * 2.0 - space::SP_1 * 2.0 - layout::ICON_BTN;
        overflowed(shown, &ends.at, room)
    }

    /// Notes where `id` ends (its right edge, window px) with no label;
    /// `true` when that changed.
    fn note_end(&self, id: &'static str, right: f32) -> bool {
        let Some(left) = self.group_left.get() else {
            return false;
        };
        let end = right - left;
        let old = self.ends.borrow_mut().at.insert(id, end);
        old.is_none_or(|o| (o - end).abs() > 0.5)
    }

    /// What the bar has over its width with no label (`None` until both
    /// are measured).
    fn room(&self) -> Option<f32> {
        Some(self.bar.get()? - self.plain.get()?)
    }
}

/// The command bar over the open message: one Tab stop, Left and Right
/// along it (test row 154), labels by priority as its width allows (row
/// 160), and what does not fit at all in More.
fn command_bar(
    view: &MailView,
    one_pane: bool,
    t: &Tokens,
    window: &mut Window,
    cx: &mut Context<MailView>,
) -> AnyElement {
    let m = view.mail.row_menu();
    let read = if m.unread {
        Command::MarkRead
    } else {
        Command::MarkUnread
    };
    let flag = if m.unflagged {
        Command::Flag
    } else {
        Command::Unflag
    };
    let entity = cx.entity().downgrade();
    let move_open = view
        .flyout
        .as_ref()
        .is_some_and(|f| f.kind == menus::FlyoutKind::Move);
    let shown: Vec<&'static str> = BAR_IDS
        .iter()
        .copied()
        .filter(|id| match *id {
            "cmd-back" => one_pane,
            "cmd-reply" | "cmd-reply-all" => !m.drafts,
            "cmd-move" => m.can_move,
            _ => true,
        })
        .collect();
    let fit = window
        .use_keyed_state("command-bar-fit", cx, |_, _| Rc::new(Fit::default()))
        .read(cx)
        .clone();
    // Each label's text, and what it adds to the bar.
    let reply_label: SharedString = text::command(Command::Reply).into();
    let forward_label: SharedString = if m.drafts {
        tr!("Forward as new draft").into()
    } else {
        text::command(Command::Forward).into()
    };
    let move_label: SharedString = tr!("Move").into();
    let label_of = |id: &str| -> Option<(SharedString, Plain)> {
        Some(match id {
            "cmd-reply" => (reply_label.clone(), Plain::Pill),
            "cmd-reply-all" => (text::command(Command::ReplyAll).into(), Plain::Icon),
            "cmd-forward" => (forward_label.clone(), Plain::Icon),
            "cmd-archive" => (text::command(Command::Archive).into(), Plain::Icon),
            "cmd-delete" => (text::command(Command::Delete).into(), Plain::Icon),
            "cmd-move" => (move_label.clone(), Plain::Menu),
            "cmd-read" => (text::command(read).into(), Plain::Icon),
            "cmd-flag" => (text::command(flag).into(), Plain::Icon),
            _ => return None,
        })
    };
    let extra: Vec<(&'static str, f32)> = shown
        .iter()
        .filter_map(|id| {
            let (label, plain) = label_of(id)?;
            Some((*id, label_extra(text_width(&label, t, window), plain, t)))
        })
        .collect();
    let labels = fit
        .room()
        .map(|room| labels_by_priority(room, &extra))
        .unwrap_or_default();
    // What the labels shown add, taken off the measured group for its
    // width with none.
    let added: f32 = extra
        .iter()
        .filter(|(id, _)| labels.contains(id))
        .map(|(_, w)| w)
        .sum();
    #[cfg(test)]
    BAR_LABELS.with(|b| *b.borrow_mut() = labels.clone());
    let labelled = |id: &str| labels.contains(&id);
    // No label fits, and the bar is still too narrow: the last buttons go
    // to More.
    let over = if labels.is_empty() {
        let buttons: Vec<&'static str> = shown
            .iter()
            .copied()
            .filter(|id| *id != "cmd-more")
            .collect();
        fit.overflow(&buttons)
    } else {
        Vec::new()
    };
    let any_label = !labels.is_empty();
    let on_bar = |id: &str| !over.contains(&id);
    let shown: Vec<&'static str> = shown.into_iter().filter(|id| on_bar(id)).collect();
    let rove = Rc::new(Roving::new(shown, window, cx));
    // A button that can go to More, with what notes where it ends.
    let measured = |id: &'static str, el: AnyElement| -> AnyElement {
        let fit = fit.clone();
        let view = entity.clone();
        div()
            .relative()
            .flex_none()
            .child(el)
            .child(
                canvas(
                    move |bounds, window, cx| {
                        if any_label {
                            return;
                        }
                        if fit.note_end(id, f32::from(bounds.right())) {
                            let v = view.clone();
                            window.defer(cx, move |_, cx| {
                                let _ = v.update(cx, |_, cx| cx.notify());
                            });
                        }
                    },
                    |_, (), _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .into_any_element()
    };
    let cmd = |id: &'static str, glyph: Icon, command: Command| Cmd::new(id, glyph, command);
    // On a server draft there is no one to reply to: Reply and Reply all
    // go, and Forward reads "Forward as new draft" (the Slint bar).
    let replies = (!m.drafts).then(|| {
        let reply = Button::new("cmd-reply")
            .icon(Icon::Reply)
            .name(reply_label.clone())
            .when(labelled("cmd-reply"), |b| b.label(reply_label.clone()))
            .tooltip(tip(Command::Reply))
            .height(Height::Medium)
            .access_key(Key::Reply)
            .on_click(cx.listener(|this, _, window, cx| {
                this.respond(
                    |message| Draft::Reply {
                        message,
                        all: false,
                    },
                    window,
                    cx,
                );
            }));
        let all = cmd("cmd-reply-all", Icon::ReplyAll, Command::ReplyAll).render(
            &rove,
            labelled("cmd-reply-all"),
            cx.listener(|this, _, window, cx| {
                this.respond(|message| Draft::Reply { message, all: true }, window, cx);
            }),
        );
        [
            Some(rove.track("cmd-reply", reply).into_any_element()),
            on_bar("cmd-reply-all").then(|| measured("cmd-reply-all", all)),
        ]
    });
    let forward = {
        let mut c = cmd("cmd-forward", Icon::Forward, Command::Forward);
        c.name = forward_label.clone();
        if m.drafts {
            c.tip = tr!("Forward as new draft (Ctrl+Shift+F)");
        }
        c.key = Some(Key::Forward);
        c
    };
    // The buttons before the gap, measured for the bar's width with no
    // label (what the labels shown add is taken off).
    let measure = {
        let fit = fit.clone();
        let view = entity.clone();
        canvas(
            move |bounds, window, cx| {
                fit.group_left.set(Some(f32::from(bounds.left())));
                // The group, the gap and More, and the bar's sides.
                let plain = f32::from(bounds.size.width) - added
                    + layout::ICON_BTN
                    + space::SP_1 * 2.0
                    + space::SP_3 * 2.0;
                if fit.plain.get().is_none_or(|p| (p - plain).abs() > 0.5) {
                    fit.plain.set(Some(plain));
                    let v = view.clone();
                    window.defer(cx, move |_, cx| {
                        let _ = v.update(cx, |_, cx| cx.notify());
                    });
                }
            },
            |_, (), _, _| {},
        )
        .absolute()
        .size_full()
    };
    let bar_measure = {
        let fit = fit.clone();
        let view = entity.clone();
        canvas(
            move |bounds, window, cx| {
                let w = f32::from(bounds.size.width);
                if fit.bar.get().is_none_or(|b| (b - w).abs() > 0.5) {
                    fit.bar.set(Some(w));
                    let v = view.clone();
                    window.defer(cx, move |_, cx| {
                        let _ = v.update(cx, |_, cx| cx.notify());
                    });
                }
            },
            |_, (), _, _| {},
        )
        .absolute()
        .size_full()
    };
    let group = div()
        .relative()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(space::SP_1))
        .child(measure)
        // One pane (below 720 px): Back to the list leads the bar.
        .when(one_pane, |el| {
            el.child(
                rove.track(
                    "cmd-back",
                    IconButton::new("cmd-back", Icon::ChevronLeft, text::back_to_list())
                        .tooltip(text::back_to_list()),
                )
                .on_click(cx.listener(|this, _, window, cx| this.back_to_list(window, cx))),
            )
        })
        .children(replies.into_iter().flatten().flatten())
        .when(on_bar("cmd-forward"), |el| {
            el.child(measured(
                "cmd-forward",
                forward.render(
                    &rove,
                    labelled("cmd-forward"),
                    cx.listener(|this, _, window, cx| {
                        let draft = |message| Draft::Forward {
                            message,
                            attached: false,
                        };
                        this.respond(draft, window, cx);
                    }),
                ),
            ))
        })
        .when(on_bar("cmd-archive"), |el| {
            let mut c = cmd("cmd-archive", Icon::Archive, Command::Archive);
            c.key = Some(Key::Archive);
            el.child(div().w(px(space::SP_3))).child(measured(
                "cmd-archive",
                c.render(
                    &rove,
                    labelled("cmd-archive"),
                    cx.listener(|this, _, window, cx| this.act(window, cx, Mail::archive)),
                ),
            ))
        })
        .when(on_bar("cmd-delete"), |el| {
            let mut c = cmd("cmd-delete", Icon::Delete, Command::Delete);
            c.key = Some(Key::Delete);
            el.child(measured(
                "cmd-delete",
                c.render(
                    &rove,
                    labelled("cmd-delete"),
                    cx.listener(|this, _, window, cx| this.act(window, cx, Mail::delete)),
                ),
            ))
        })
        .when(on_bar("cmd-move"), |el| {
            // A menu button, as the Slint bar's (`menu: true`): its chevron,
            // and expanded while the Move flyout is open.
            el.child(measured(
                "cmd-move",
                rove.track(
                    "cmd-move",
                    Button::new("cmd-move")
                        .icon(Icon::MoveTo)
                        .when(labelled("cmd-move"), |b| b.label(move_label.clone()))
                        .trailing(Icon::ChevronDown)
                        .name(move_label.clone())
                        .variant(Variant::Subtle)
                        .height(Height::Medium)
                        .tooltip(tr!("Move (Ctrl+Shift+V)"))
                        .disabled(!m.can_move)
                        .aria_expanded(move_open),
                )
                .on_click(cx.listener(|this, _, window, cx| menus::open_move(this, window, cx)))
                .into_any_element(),
            ))
        })
        .when(on_bar("cmd-read"), |el| {
            let glyph = if m.unread { Icon::MailRead } else { Icon::Mail };
            let mut c = cmd("cmd-read", glyph, read);
            c.key = Some(if m.unread {
                Key::MarkRead
            } else {
                Key::MarkUnread
            });
            el.child(div().w(px(space::SP_3))).child(measured(
                "cmd-read",
                c.render(
                    &rove,
                    labelled("cmd-read"),
                    cx.listener(move |this, _, window, cx| {
                        let on = read == Command::MarkRead;
                        this.act(window, cx, |m| m.mark(Flag::Seen, on));
                    }),
                ),
            ))
        })
        .when(on_bar("cmd-flag"), |el| {
            let mut c = cmd("cmd-flag", Icon::Important, flag);
            c.toggled = Some(flag == Command::Unflag);
            el.child(measured(
                "cmd-flag",
                c.render(
                    &rove,
                    labelled("cmd-flag"),
                    cx.listener(move |this, _, window, cx| {
                        let on = flag == Command::Flag;
                        this.act(window, cx, |m| m.mark(Flag::Flagged, on));
                    }),
                ),
            ))
        });
    let keys = rove.clone();
    div()
        .id("command-bar")
        .role(Role::Toolbar)
        .aria_label(tr!("Message commands"))
        .relative()
        .h(px(layout::CMD_BAR_H))
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(space::SP_1))
        .px(px(space::SP_3))
        .overflow_hidden()
        .text_color(t.color.ink)
        .on_key_down(move |e: &KeyDownEvent, window, cx| {
            if e.keystroke.modifiers.modified() {
                return;
            }
            let Some(at) = keys.handles.iter().position(|h| h.is_focused(window)) else {
                return;
            };
            let Some(to) = bar_step(e.keystroke.key.as_str(), at, keys.handles.len()) else {
                return;
            };
            cx.stop_propagation();
            window.focus(&keys.handles[to], cx);
        })
        .child(bar_measure)
        .child(group)
        .child(div().flex_1())
        .child(
            rove.track(
                "cmd-more",
                IconButton::new("cmd-more", Icon::More, text::more_actions()),
            )
            .dropdown_menu(move |menu, window, cx| {
                menus::more_menu(entity.clone(), &over, menu, window, cx)
            }),
        )
        .test_support()
        .into_any_element()
}

/// Several messages selected, none open: their count and actions (the
/// Slint selection panel): Archive, Delete, Move (with why it is off),
/// then Mark as read and as unread, Mark as important and Remove
/// importance, two to a row so the labels fit a narrow pane, then Select
/// all and Clear selection.
fn selection(view: &MailView, t: &Tokens, cx: &mut Context<MailView>) -> AnyElement {
    let n = view.mail.selection().len();
    let m = view.mail.row_menu();
    let said = text::selected(n);
    let row = || {
        div()
            .flex()
            .flex_row()
            .justify_center()
            .gap(px(space::SP_1))
    };
    let command = |id: &'static str, icon: Icon, cmd: Command| {
        Button::new(id)
            .icon(icon)
            .label(text::command(cmd))
            .tooltip(text::command_tip(cmd))
    };
    div()
        .id("selection")
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(space::SP_3))
        .px(px(space::SP_4))
        .child(
            div()
                .id("selection-count")
                .role(Role::Status)
                .aria_label(said.clone())
                .typo(&t.text.subtitle)
                .text_color(t.color.ink)
                .child(said)
                .test_support(),
        )
        .child(
            row()
                .child(
                    command("sel-archive", Icon::Archive, Command::Archive).on_click(
                        cx.listener(|this, _, window, cx| this.act(window, cx, Mail::archive)),
                    ),
                )
                .child(
                    command("sel-delete", Icon::Delete, Command::Delete).on_click(
                        cx.listener(|this, _, window, cx| this.act(window, cx, Mail::delete)),
                    ),
                )
                .child(
                    Button::new("sel-move")
                        .icon(Icon::MoveTo)
                        .label(text::move_label())
                        .tooltip(text::move_tip())
                        .disabled(!m.can_move)
                        .on_click(cx.listener(|this, _, window, cx| {
                            menus::open_move(this, window, cx);
                        })),
                ),
        )
        // Why Move is off, said where it shows (a disabled button shows no
        // tooltip).
        .when(!m.can_move, |el| {
            let why = text::move_one_account();
            el.child(
                div()
                    .id("sel-move-why")
                    .role(Role::Label)
                    .aria_label(why.clone())
                    .max_w(px(layout::LIST_W_COMPACT))
                    .text_center()
                    .typo(&t.text.caption)
                    .text_color(t.color.ink_2)
                    .child(why)
                    .test_support(),
            )
        })
        .child(
            row()
                .child(
                    command("sel-read", Icon::MailRead, Command::MarkRead).on_click(cx.listener(
                        |this, _, window, cx| this.act(window, cx, |m| m.mark(Flag::Seen, true)),
                    )),
                )
                .child(
                    command("sel-unread", Icon::Mail, Command::MarkUnread).on_click(cx.listener(
                        |this, _, window, cx| this.act(window, cx, |m| m.mark(Flag::Seen, false)),
                    )),
                ),
        )
        .child(
            row()
                .child(
                    command("sel-flag", Icon::Flag, Command::Flag).on_click(cx.listener(
                        |this, _, window, cx| this.act(window, cx, |m| m.mark(Flag::Flagged, true)),
                    )),
                )
                .child(
                    command("sel-unflag", Icon::Flag, Command::Unflag).on_click(cx.listener(
                        |this, _, window, cx| {
                            this.act(window, cx, |m| m.mark(Flag::Flagged, false))
                        },
                    )),
                ),
        )
        .child(
            row()
                .child(
                    Button::new("sel-all")
                        .variant(crate::widgets::button::Variant::Subtle)
                        .label(text::select_all())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.act(window, cx, Mail::select_all)
                        })),
                )
                .child(
                    Button::new("sel-clear")
                        .variant(crate::widgets::button::Variant::Subtle)
                        .label(text::clear_selection())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.act(window, cx, Mail::clear_selection)
                        })),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui::{Role, TestAppContext};
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::test::TestWindowExt as _;

    use super::super::menus::{self, Run};
    use super::super::text::Command;
    use crate::test_support::{self, assert_named, in_window, press};
    use crate::views::composer::Draft;
    use gpui::Focusable as _;

    /// The Slint reader bar's commands (ui/app.slint, the command bar):
    /// Reply, Reply all and Forward; Archive, Delete and Move; Mark as read
    /// or unread and Mark as important; More. The More menu is the Slint
    /// `more_menu`'s (the reader's view toggles and the source commands,
    /// which run: Show original opens its dialog).
    #[gpui_kit::test]
    fn the_command_bar_and_the_more_menu_of_the_open_message(cx: &mut TestAppContext) {
        test_support::init_app(cx);
        let (_cache, db, seeded) = test_support::seeded("reading-bar", 3);
        db.set_setting("mark_read_delay_ms", "0").unwrap();
        let ids = db.message_ids(&[seeded.inbox], "").unwrap();
        let (window, main) = test_support::open_main(cx, test_support::init(Some(db)));
        let view = main.read_with(cx, |w, _| w.mail().cloned().expect("the mail view"));
        in_window(cx, window, |w, cx| {
            view.update(cx, |v, cx| v.act(w, cx, |m| m.row_clicked(ids[0])))
        });
        in_window(cx, window, |w, _| {
            for (id, name) in [
                ("cmd-reply", "Reply"),
                ("cmd-reply-all", "Reply all"),
                ("cmd-forward", "Forward"),
                ("cmd-archive", "Archive"),
                ("cmd-delete", "Delete"),
                ("cmd-move", "Move"),
                ("cmd-flag", "Mark as important"),
                ("cmd-more", "More actions"),
            ] {
                let b = w.find(id);
                assert_eq!(
                    (b.label(), b.role()),
                    (Some(name), Some(Role::Button)),
                    "{id}"
                );
            }
        });
        let groups = view.read_with(cx, menus::more_entries);
        let labels: Vec<Vec<(String, bool)>> = groups
            .iter()
            .map(|g| g.iter().map(|e| (e.label.clone(), e.enabled)).collect())
            .collect();
        assert_eq!(
            labels[0],
            [
                ("Open in new window".to_string(), true),
                ("Forward as attachment".to_string(), true)
            ]
        );
        assert!(labels[2].iter().any(|(l, _)| l == "View as plain text"));
        assert_eq!(
            labels[3],
            [
                ("Save as .eml".to_string(), true),
                ("Show original".to_string(), true),
                ("Copy Message-ID".to_string(), true)
            ]
        );
        in_window(cx, window, |w, cx| {
            view.update(cx, |v, cx| v.run(Run::Cmd(Command::ShowOriginal), w, cx))
        });
        cx.run_until_parked();
        in_window(cx, window, |w, cx| {
            assert!(w.has_active_dialog(cx));
            assert_named(w, "Copy all", Role::Button);
        });
    }

    /// Row 154, the Slint app's `tab_reaches_the_reader_toolbar_as_one_stop`:
    /// Shift+Tab from the message reaches the bar at one button; Left,
    /// Right, Home and End go along it (wrapping); Tab leaves it for the
    /// message, and Shift+Tab comes back to the button last used.
    #[gpui_kit::test]
    fn tab_reaches_the_reader_toolbar_as_one_stop(cx: &mut TestAppContext) {
        test_support::init_app(cx);
        let (_cache, db, seeded) = test_support::seeded("reading-roving", 3);
        db.set_setting("mark_read_delay_ms", "0").unwrap();
        let ids = db.message_ids(&[seeded.inbox], "").unwrap();
        let (window, main) = test_support::open_main(cx, test_support::init(Some(db)));
        let view = main.read_with(cx, |w, _| w.mail().cloned().expect("the mail view"));
        in_window(cx, window, |w, cx| {
            view.update(cx, |v, cx| v.act(w, cx, |m| m.row_clicked(ids[0])))
        });
        let page = view.read_with(cx, |v, cx| v.reader.read(cx).focus_handle(cx));
        let focused = |cx: &mut TestAppContext| -> Option<&'static str> {
            in_window(cx, window, |w, _| {
                super::BAR_FOCUS.with(|b| {
                    b.borrow()
                        .iter()
                        .find(|(_, h)| h.is_focused(w))
                        .map(|(id, _)| *id)
                })
            })
        };
        let keys = |cx: &mut TestAppContext, keys: &[&str]| {
            in_window(cx, window, |w, cx| press(w, keys, cx));
        };
        in_window(cx, window, |w, cx| w.focus(&page, cx));
        keys(cx, &["shift-tab"]);
        assert_eq!(focused(cx), Some("cmd-reply"), "the bar's one stop");
        keys(cx, &["right"]);
        assert_eq!(focused(cx), Some("cmd-reply-all"));
        keys(cx, &["end"]);
        assert_eq!(focused(cx), Some("cmd-more"));
        keys(cx, &["right"]);
        assert_eq!(focused(cx), Some("cmd-reply"), "Right wraps");
        keys(cx, &["left", "left"]);
        assert_eq!(focused(cx), Some("cmd-flag"));
        keys(cx, &["home", "end", "left"]);
        assert_eq!(focused(cx), Some("cmd-flag"));
        // Tab leaves the bar (no stop on More) for the message.
        keys(cx, &["tab"]);
        assert_eq!(focused(cx), None);
        assert!(in_window(cx, window, |w, _| page.is_focused(w)));
        keys(cx, &["shift-tab"]);
        assert_eq!(focused(cx), Some("cmd-flag"), "the button last used");
        // The roving keys are the bar's only: no arrow moved the list.
        assert_eq!(view.read_with(cx, |v, _| v.mail.open_id()), Some(ids[0]));
    }

    /// Row 164, the Slint app's `tab_from_a_row_skips_its_quick_actions`:
    /// the quick actions on the keyboard's row are no Tab stops, so Tab from
    /// the ringed row goes to the reader's bar (Reply), not into the row;
    /// Enter there replies.
    #[gpui_kit::test]
    fn tab_from_a_row_skips_its_quick_actions(cx: &mut TestAppContext) {
        test_support::init_app(cx);
        let (_cache, db, seeded) = test_support::seeded("reading-tab-quick", 3);
        db.set_setting("mark_read_delay_ms", "0").unwrap();
        let ids = db.message_ids(&[seeded.inbox], "").unwrap();
        let (window, main) = test_support::open_main(cx, test_support::init(Some(db)));
        let view = main.read_with(cx, |w, _| w.mail().cloned().expect("the mail view"));
        let list = view.read_with(cx, |v, _| v.list_focus.clone());
        in_window(cx, window, |w, cx| w.focus(&list, cx));
        // End opens the last row, with the keyboard's ring and its quick
        // actions on it.
        in_window(cx, window, |w, cx| press(w, &["end"], cx));
        cx.run_until_parked();
        let last = *ids.last().expect("rows");
        assert_eq!(view.read_with(cx, |v, _| v.mail.open_id()), Some(last));
        let quick = in_window(cx, window, |w, _| {
            test_support::names(w)
                .into_iter()
                .filter(|(_, name)| name == "Archive")
                .count()
        });
        assert!(quick >= 2, "the row's Archive and the bar's: {quick}");
        in_window(cx, window, |w, cx| press(w, &["tab"], cx));
        let focused = in_window(cx, window, |w, _| {
            super::BAR_FOCUS.with(|b| {
                b.borrow()
                    .iter()
                    .find(|(_, h)| h.is_focused(w))
                    .map(|(id, _)| *id)
            })
        });
        assert_eq!(focused, Some("cmd-reply"), "Tab went to Reply");
        in_window(cx, window, |w, cx| press(w, &["enter"], cx));
        cx.run_until_parked();
        let draft = view.read_with(cx, |v, cx| v.composer.read(cx).draft());
        assert!(
            matches!(draft, Some(Draft::Reply { all: false, .. })),
            "{draft:?}"
        );
    }

    /// Row 160, the Slint app's `reader_bar_labels_go_by_priority`: the
    /// labels go by priority, each measured on its own. A wide window
    /// (1920 x 800) labels Reply, Archive and Delete in English and pt-PT;
    /// 1280 x 800 at least Reply, in both; narrower, labels go (Reply the
    /// last), then the buttons are icons (Reply still named Reply) and the
    /// last of them leave for More, with nothing clipped.
    #[gpui_kit::test]
    fn reader_bar_labels_go_by_priority(cx: &mut TestAppContext) {
        test_support::init_app(cx);
        let (_cache, db, seeded) = test_support::seeded("reading-labels", 3);
        db.set_setting("mark_read_delay_ms", "0").unwrap();
        let ids = db.message_ids(&[seeded.inbox], "").unwrap();
        let (window, main) = test_support::open_main(cx, test_support::init(Some(db)));
        let view = main.read_with(cx, |w, _| w.mail().cloned().expect("the mail view"));
        in_window(cx, window, |w, cx| {
            view.update(cx, |v, cx| v.act(w, cx, |m| m.row_clicked(ids[0])))
        });
        let settle = |cx: &mut TestAppContext| {
            for _ in 0..4 {
                in_window(cx, window, |_, _| {});
                cx.run_until_parked();
            }
        };
        let labels = |cx: &mut TestAppContext| {
            in_window(cx, window, |_, _| {
                super::BAR_LABELS.with(|b| b.borrow().clone())
            })
        };
        let width = |cx: &mut TestAppContext, id: &'static str| {
            in_window(cx, window, |w, _| {
                let b = w.find(id);
                (
                    b.label().map(str::to_string),
                    f32::from(b.bounds().size.width),
                )
            })
        };
        // The buttons in the bar's arrow order, and whether each one's
        // right edge is inside the bar (none clipped out of sight).
        let on_bar = |cx: &mut TestAppContext| {
            in_window(cx, window, |w, _| {
                let right = f32::from(w.find("command-bar").bounds().right());
                super::BAR_FOCUS.with(|b| {
                    b.borrow()
                        .iter()
                        .map(|(id, _)| {
                            (*id, f32::from(w.find(*id).bounds().right()) <= right + 0.5)
                        })
                        .collect::<Vec<_>>()
                })
            })
        };
        let ids = |shown: &[(&'static str, bool)]| shown.iter().map(|s| s.0).collect::<Vec<_>>();
        // The order holds: a label only with those before it in the strict
        // three (Reply, Archive, Delete), the rest only after them.
        let in_order = |got: &[&'static str], at: &str| {
            let has = |id: &str| got.contains(&id);
            if has("cmd-archive") {
                assert!(has("cmd-reply"), "{at}: {got:?}");
            }
            if has("cmd-delete") {
                assert!(has("cmd-archive"), "{at}: {got:?}");
            }
            if super::THEN.iter().any(|id| has(id)) {
                assert!(has("cmd-delete"), "{at}: {got:?}");
            }
        };
        let icon = crate::theme::layout::ICON_BTN;
        let big = gpui::size(gpui::px(1920.), gpui::px(800.));
        cx.simulate_window_resize(window, big);
        settle(cx);
        let en = labels(cx);
        for id in ["cmd-reply", "cmd-archive", "cmd-delete"] {
            assert!(en.contains(&id), "English at 1920: {en:?}");
        }
        in_order(&en, "English at 1920");
        let (name, wide) = width(cx, "cmd-archive");
        assert_eq!(name.as_deref(), Some("Archive"));
        assert!(wide > icon * 1.5, "Archive labelled: {wide}");
        let shown = on_bar(cx);
        assert_eq!(ids(&shown).len(), super::BAR_IDS.len() - 1, "all but Back");
        assert!(shown.iter().all(|s| s.1), "{shown:?}");
        // Narrower: labels go, the last kept is always Reply.
        let mut last = en.len();
        for px in [1600., 1440., 1280., 1180., 1100., 1000., 900., 800., 720.] {
            cx.simulate_window_resize(window, gpui::size(gpui::px(px), gpui::px(800.)));
            settle(cx);
            let got = labels(cx);
            in_order(&got, &format!("{px}"));
            assert!(got.len() <= last, "{px}: {got:?} after {last}");
            last = got.len();
            let shown = on_bar(cx);
            assert!(shown.iter().all(|s| s.1), "{px}: {shown:?}");
            if px == 1280. {
                assert!(got.contains(&"cmd-reply"), "1280: {got:?}");
            }
        }
        cx.simulate_window_resize(window, gpui::size(gpui::px(420.), gpui::px(800.)));
        settle(cx);
        assert!(labels(cx).is_empty(), "no label at 420 px");
        let (name, narrow) = width(cx, "cmd-reply");
        assert_eq!(name.as_deref(), Some("Reply"), "still named");
        assert!(narrow < icon * 1.5, "an icon when narrow: {narrow}");
        // Back leads; what does not fit leaves the bar for More, which
        // stays in sight, and nothing in the arrow order is clipped.
        let shown = on_bar(cx);
        let kept = ids(&shown);
        assert!(shown.iter().all(|s| s.1), "{shown:?}");
        assert_eq!(kept.first(), Some(&"cmd-back"));
        assert_eq!(kept.last(), Some(&"cmd-more"));
        assert!(!kept.contains(&"cmd-flag"), "{kept:?}");
        cx.simulate_window_resize(window, big);
        settle(cx);
        assert_eq!(labels(cx), en, "the labels come back");
        assert_eq!(
            ids(&on_bar(cx)).len(),
            super::BAR_IDS.len() - 1,
            "and so do the buttons"
        );
        // In pt-PT, measured again for its own labels (longer words: no
        // more labels than in English).
        test_support::set_lang(cx, komorebi::i18n::Lang::PtPt);
        settle(cx);
        let pt = labels(cx);
        for id in ["cmd-reply", "cmd-archive", "cmd-delete"] {
            assert!(pt.contains(&id), "pt-PT at 1920: {pt:?}");
        }
        in_order(&pt, "pt-PT at 1920");
        assert!(pt.len() <= en.len(), "{pt:?} {en:?}");
        assert_eq!(width(cx, "cmd-reply").0.as_deref(), Some("Responder"));
        // The default window keeps Reply's label in pt-PT too.
        cx.simulate_window_resize(window, gpui::size(gpui::px(1280.), gpui::px(800.)));
        settle(cx);
        let pt_1280 = labels(cx);
        assert!(pt_1280.contains(&"cmd-reply"), "pt-PT at 1280: {pt_1280:?}");
        in_order(&pt_1280, "pt-PT at 1280");
        cx.simulate_window_resize(window, gpui::size(gpui::px(420.), gpui::px(800.)));
        settle(cx);
        assert!(labels(cx).is_empty(), "no label in pt-PT when narrow");
        let shown = on_bar(cx);
        assert!(shown.iter().all(|s| s.1), "{shown:?}");
        assert_eq!(ids(&shown).last(), Some(&"cmd-more"));
        test_support::set_lang(cx, komorebi::i18n::Lang::En);
    }

    /// The priority itself: Reply, Archive and Delete strictly in that
    /// order, then each of the rest that still fits; a button not shown is
    /// skipped.
    #[test]
    fn labels_go_by_priority_each_measured_on_its_own() {
        let extra = [
            ("cmd-reply", 40.0),
            ("cmd-reply-all", 70.0),
            ("cmd-forward", 60.0),
            ("cmd-archive", 55.0),
            ("cmd-delete", 50.0),
            ("cmd-move", 45.0),
            ("cmd-read", 90.0),
            ("cmd-flag", 120.0),
        ];
        let by = |room: f32| super::labels_by_priority(room, &extra);
        assert!(by(39.0).is_empty(), "Reply first or nothing");
        assert_eq!(by(40.0), ["cmd-reply"]);
        assert_eq!(by(95.0), ["cmd-reply", "cmd-archive"]);
        // Room for Archive's 55 but not Delete's 50 after it: Delete and the
        // rest wait.
        assert_eq!(by(144.0), ["cmd-reply", "cmd-archive"]);
        assert_eq!(by(145.0), ["cmd-reply", "cmd-archive", "cmd-delete"]);
        // After the three, 110 left: Mark as read (90) fits, then Mark as
        // important (120) does not, Move (45) does not after it, Reply all
        // (70) neither, Forward (60) neither.
        assert_eq!(
            by(145.0 + 110.0),
            ["cmd-reply", "cmd-archive", "cmd-delete", "cmd-read"]
        );
        // 85 left: Mark as read and Mark as important do not fit, Move
        // does, then Reply all and Forward do not.
        assert_eq!(
            by(145.0 + 85.0),
            ["cmd-reply", "cmd-archive", "cmd-delete", "cmd-move"]
        );
        assert_eq!(by(10_000.0).len(), extra.len());
        // A server draft: no Reply, so Delete is enough for the rest.
        let draft: Vec<_> = extra
            .iter()
            .copied()
            .filter(|(id, _)| !id.starts_with("cmd-reply"))
            .collect();
        assert_eq!(
            super::labels_by_priority(150.0, &draft),
            ["cmd-archive", "cmd-delete", "cmd-move"]
        );
    }

    /// The buttons that go to More: from the first that passes the room to
    /// the end, never Back or Reply; one not measured yet fits.
    #[test]
    fn the_last_buttons_go_to_more() {
        let shown = [
            "cmd-back",
            "cmd-reply",
            "cmd-reply-all",
            "cmd-forward",
            "cmd-archive",
            "cmd-delete",
        ];
        let ends: std::collections::HashMap<&'static str, f32> = [
            ("cmd-back", 36.0),
            ("cmd-reply", 76.0),
            ("cmd-reply-all", 116.0),
            ("cmd-forward", 156.0),
            ("cmd-archive", 206.0),
        ]
        .into_iter()
        .collect();
        assert!(super::overflowed(&shown, &ends, 400.0).is_empty());
        assert_eq!(
            super::overflowed(&shown, &ends, 160.0),
            ["cmd-archive", "cmd-delete"]
        );
        assert_eq!(
            super::overflowed(&shown, &ends, 20.0),
            ["cmd-reply-all", "cmd-forward", "cmd-archive", "cmd-delete"]
        );
    }

    /// The buttons that went to More are its first entries, the same
    /// commands as their buttons.
    #[gpui_kit::test]
    fn the_more_menu_starts_with_what_left_the_bar(cx: &mut TestAppContext) {
        test_support::init_app(cx);
        let (_cache, db, seeded) = test_support::seeded("reading-overflow", 2);
        let ids = db.message_ids(&[seeded.inbox], "").unwrap();
        let (window, main) = test_support::open_main(cx, test_support::init(Some(db)));
        let view = main.read_with(cx, |w, _| w.mail().cloned().expect("the mail view"));
        in_window(cx, window, |w, cx| {
            view.update(cx, |v, cx| v.act(w, cx, |m| m.row_clicked(ids[0])))
        });
        let entries = view.read_with(cx, |v, _| {
            menus::bar_entries(v, &["cmd-archive", "cmd-move", "cmd-flag", "cmd-more"])
        });
        let runs: Vec<_> = entries.iter().map(|e| e.run).collect();
        assert_eq!(runs.len(), 3, "{entries:?}");
        assert_eq!(
            runs[..2],
            [Run::Cmd(Command::Archive), Run::Cmd(Command::MoveTo)]
        );
        assert!(matches!(
            runs[2],
            Run::Cmd(Command::Flag) | Run::Cmd(Command::Unflag)
        ));
    }
}
