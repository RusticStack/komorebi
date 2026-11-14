//! One message at full width, in a tab of the main window or in a window
//! of its own (docs/GPUI-MIGRATION.md phase 5; the Slint app's message tab
//! and `src/app/windows.rs` popout): the command bar (mockup `.cmd`, the
//! reading pane's groups: Reply the only labelled pill, then Reply all and
//! Forward; Archive, Delete and Move; Mark as read or unread and Mark as
//! important; Open in new window in a tab; More: Forward as attachment,
//! Report junk or Not junk, the reader's view toggles and the source) over
//! its own reader. Once the message is moved or deleted on the server, the
//! notice says so and the actions are off; the cached copy stays readable
//! until the tab or window closes.
//!
//! The view acts on nothing itself: it asks its owner ([`MessageEvent`]),
//! which routes replies to a draft tab and actions to the mail view, so a
//! message window's Archive is the main window's Archive (with its Undo).

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, Role, Styled, Subscription, WeakEntity,
    Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use komorebi::role::Role as FolderRole;

use super::text;
use crate::access::AccessHost as _;
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, layout, space};
use crate::views::Db;
use crate::views::mail::text::{self as mail_text, Command};
use crate::views::mail::triage::{self, MoveOption};
use crate::views::reader::actions::{self as reader_actions, MenuCmd, SourceUse};
use crate::views::reader::{ReaderEvent, ReaderView};
use crate::widgets::button::{Button, Height, IconButton, Variant};
use crate::widgets::menu;
use crate::widgets::notice::{Kind, Notice};
use crate::widgets::pane::Pane;

gpui::actions!(komorebi_message, [CloseMessage]);

/// The key context of a message window (Esc closes it).
pub const WINDOW_CONTEXT: &str = "MessageWindow";

/// What a message view asks of its owner.
#[derive(Clone, Debug, PartialEq)]
pub enum MessageEvent {
    Reply {
        message: i64,
        all: bool,
    },
    Forward {
        message: i64,
    },
    /// More > Forward as attachment.
    ForwardAttached(i64),
    Archive(i64),
    Delete(i64),
    /// Move to this folder.
    Move {
        message: i64,
        folder: i64,
    },
    /// Report junk.
    Junk(i64),
    /// Not junk (a message in Junk).
    NotJunk(i64),
    /// Mark as important or Remove importance (turned over).
    Flag(i64),
    /// Mark as read or unread (turned over).
    Read(i64),
    OpenWindow(i64),
    /// Esc in a message window, or its Close.
    Close,
    /// What the reader asks that its owner does (downloads, read marks,
    /// links, the status line).
    Reader(ReaderEvent),
}

pub struct MessageView {
    db: Db,
    id: i64,
    reader: Entity<ReaderView>,
    /// The message left the server (or the cache): actions off.
    gone: bool,
    /// In a window of its own (no Open in new window; Esc closes).
    window: bool,
    /// The reader opened its message (a background tab waits until shown).
    opened: bool,
    /// The command bar's state, read from the cache when the view is made
    /// and when its owner says the cache changed ([`Self::refresh`]), not
    /// on every frame.
    bar: Bar,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<MessageEvent> for MessageView {}

/// What the command bar shows for the message, read from the cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bar {
    pub seen: bool,
    pub flagged: bool,
    /// It is in its account's Junk folder: Not junk, else Report junk.
    pub junk: bool,
    /// It has a folder to move to.
    pub can_move: bool,
}

/// What a More menu entry does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum More {
    ForwardAttached,
    Junk,
    NotJunk,
    /// A view toggle of the reader.
    Reader(MenuCmd),
    /// Save as .eml, Show original, Copy Message-ID (the reader runs them).
    Source(SourceUse),
}

/// One entry of the More menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoreEntry {
    pub label: String,
    pub run: More,
    pub enabled: bool,
    pub checked: Option<bool>,
}

impl MessageView {
    /// Message `id`, in a tab (`in_window` false) or a window of its own.
    /// The reader opens it when the view first shows.
    pub fn new(
        db: Db,
        id: i64,
        in_window: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let reader = cx.new(|cx| ReaderView::new(db.clone(), window, cx));
        // Its "You started a reply" bar opens in a tab (the window's
        // `tab_reader`).
        reader.update(cx, |r, cx| r.handle_resume(cx));
        crate::access::release_on_deactivate(window, cx);
        let subscriptions = vec![cx.subscribe(&reader, |_, _, event: &ReaderEvent, cx| {
            cx.emit(MessageEvent::Reader(event.clone()));
        })];
        let bar = read_bar(&db, id);
        Self {
            db,
            id,
            reader,
            gone: false,
            window: in_window,
            opened: false,
            bar,
            // Tab reaches the view (its command bar, then its reader).
            focus: cx.focus_handle().tab_stop(true),
            _subscriptions: subscriptions,
        }
    }

    /// The cache changed (a mark, a move, a sync): the bar reads it again,
    /// and draws again when it differs.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let bar = read_bar(&self.db, self.id);
        if bar != self.bar {
            self.bar = bar;
            cx.notify();
        }
    }

    pub fn message(&self) -> i64 {
        self.id
    }

    pub fn reader(&self) -> &Entity<ReaderView> {
        &self.reader
    }

    pub fn is_gone(&self) -> bool {
        self.gone
    }

    /// The message is no longer on the server (a removal, a vanished
    /// folder, a removed account): the notice shows, actions go off.
    pub fn set_gone(&mut self, gone: bool, cx: &mut Context<Self>) {
        if self.gone != gone {
            self.gone = gone;
            cx.notify();
        }
    }

    /// Whether the cache still holds the message.
    pub fn cached(&self) -> bool {
        matches!(self.db.message(self.id), Ok(Some(_)))
    }

    /// The reader shows the message (on the first frame the view shows).
    pub fn ensure_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.opened {
            return;
        }
        self.opened = true;
        let id = self.id;
        let shown = self.reader.update(cx, |r, cx| r.open(id, window, cx));
        if !shown {
            self.gone = true;
        }
    }

    /// What the command bar shows, as last read from the cache (all off
    /// once the message has left it).
    pub fn bar(&self) -> Bar {
        self.bar
    }

    fn brief(&self) -> Option<komorebi::db::Brief> {
        brief(&self.db, self.id)
    }

    /// The folders the message can move to: the recent ones first (as the
    /// mail view's Move flyout lists them, with no filter), then all of its
    /// account's but its own.
    pub fn move_options(&self) -> Vec<MoveOption> {
        let Some(m) = self.brief() else {
            return Vec::new();
        };
        let folders = self.db.folders().unwrap_or_default();
        let options = triage::move_options_in(&folders, m.account_id, &[m.folder_id]);
        if options.is_empty() {
            return options;
        }
        let recent = self
            .db
            .settings()
            .map(|s| triage::parse_recent(&s.move_recent))
            .unwrap_or_default();
        triage::move_entries(&options, &recent, "")
    }

    /// The More menu's groups (the reading pane's More menu, less what the
    /// bar has already): Forward as attachment; Report junk or Not junk;
    /// the reader's view toggles; the source commands.
    pub fn more_entries(&self, cx: &App) -> Vec<Vec<MoreEntry>> {
        let on = !self.gone;
        let bar = self.bar();
        let cmd = |c: Command, run: More| MoreEntry {
            label: mail_text::command(c),
            run,
            enabled: on,
            checked: None,
        };
        let junk = if bar.junk {
            cmd(Command::NotJunk, More::NotJunk)
        } else {
            cmd(Command::Junk, More::Junk)
        };
        let v = self.reader.read(cx).view_menu();
        let toggle = |c: MenuCmd, (enabled, checked): (bool, bool)| MoreEntry {
            label: reader_actions::menu_label(c),
            run: More::Reader(c),
            enabled,
            checked: Some(checked),
        };
        let images = if v.stop_images {
            MenuCmd::StopImages
        } else {
            MenuCmd::LoadImages
        };
        vec![
            vec![cmd(Command::ForwardAttached, More::ForwardAttached)],
            vec![junk],
            vec![
                toggle(MenuCmd::Plain, v.plain),
                toggle(MenuCmd::Colours, v.colours),
                MoreEntry {
                    checked: None,
                    ..toggle(images, (v.stop_images || v.load_images, false))
                },
            ],
            vec![
                cmd(Command::SaveEml, More::Source(SourceUse::SaveEml)),
                cmd(Command::ShowOriginal, More::Source(SourceUse::ShowOriginal)),
                cmd(Command::CopyId, More::Source(SourceUse::CopyId)),
            ],
        ]
    }

    /// Run a More menu entry: the reader's own entries here, the rest asked
    /// of the owner.
    pub fn run_more(&mut self, run: More, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.id;
        match run {
            More::ForwardAttached => self.ask(MessageEvent::ForwardAttached(id), cx),
            More::Junk => self.ask(MessageEvent::Junk(id), cx),
            More::NotJunk => self.ask(MessageEvent::NotJunk(id), cx),
            More::Reader(c) => self.reader.update(cx, |r, cx| r.menu(c, window, cx)),
            More::Source(what) => self.reader.update(cx, |r, cx| r.source(what, window, cx)),
        }
    }

    /// Ask the owner for `event`; the bar reads the cache again after it.
    pub fn ask(&mut self, event: MessageEvent, cx: &mut Context<Self>) {
        cx.emit(event);
        cx.notify();
    }

    fn command_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let id = self.id;
        let off = self.gone;
        let bar = self.bar();
        let icon = |key: &'static str, glyph: Icon, cmd: Command| {
            IconButton::new(key, glyph, mail_text::command(cmd))
                .tooltip(mail_text::command_tip(cmd))
                .disabled(off)
        };
        let read = if bar.seen {
            Command::MarkUnread
        } else {
            Command::MarkRead
        };
        let flag = if bar.flagged {
            Command::Unflag
        } else {
            Command::Flag
        };
        let move_entity = cx.entity().downgrade();
        let more_entity = move_entity.clone();
        div()
            .id("message-commands")
            .role(Role::Toolbar)
            .h(px(layout::CMD_BAR_H))
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(space::SP_1))
            .px(px(space::SP_3))
            .child(
                Button::new("message-reply")
                    .icon(Icon::Reply)
                    .label(mail_text::command(Command::Reply))
                    .height(Height::Medium)
                    .disabled(off)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ask(
                            MessageEvent::Reply {
                                message: id,
                                all: false,
                            },
                            cx,
                        )
                    })),
            )
            .child(
                icon("message-reply-all", Icon::ReplyAll, Command::ReplyAll).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.ask(
                            MessageEvent::Reply {
                                message: id,
                                all: true,
                            },
                            cx,
                        )
                    },
                )),
            )
            .child(
                icon("message-forward", Icon::Forward, Command::Forward).on_click(cx.listener(
                    move |this, _, _, cx| this.ask(MessageEvent::Forward { message: id }, cx),
                )),
            )
            .child(div().w(px(space::SP_2_5)))
            .child(
                icon("message-archive", Icon::Archive, Command::Archive).on_click(
                    cx.listener(move |this, _, _, cx| this.ask(MessageEvent::Archive(id), cx)),
                ),
            )
            .child(
                icon("message-delete", Icon::Delete, Command::Delete).on_click(
                    cx.listener(move |this, _, _, cx| this.ask(MessageEvent::Delete(id), cx)),
                ),
            )
            .child(
                // A menu button, as the reading pane's: the message's
                // folders, the recent ones first.
                Button::new("message-move")
                    .icon(Icon::Folder)
                    .trailing(Icon::ChevronDown)
                    .name(mail_text::move_label())
                    .variant(Variant::Subtle)
                    .height(Height::Medium)
                    .tooltip(mail_text::move_label())
                    .disabled(off || !bar.can_move)
                    .dropdown_menu(move |menu, window, cx| {
                        move_menu(move_entity.clone(), menu, window, cx)
                    }),
            )
            .child(div().w(px(space::SP_2_5)))
            .child(
                icon(
                    "message-read",
                    if bar.seen { Icon::Mail } else { Icon::MailRead },
                    read,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.ask(MessageEvent::Read(id), cx))),
            )
            .child(
                icon("message-flag", Icon::Important, flag)
                    .toggled(bar.flagged)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.ask(MessageEvent::Flag(id), cx)),
                    ),
            )
            .child(div().flex_1())
            .when(!self.window, |el| {
                el.child(
                    IconButton::new("message-window", Icon::Open, text::open_window()).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.ask(MessageEvent::OpenWindow(id), cx)
                        }),
                    ),
                )
            })
            .child(
                IconButton::new("message-more", Icon::More, mail_text::more_actions())
                    .disabled(off)
                    .dropdown_menu(move |menu, window, cx| {
                        more_menu(more_entity.clone(), menu, window, cx)
                    }),
            )
            .test_support()
    }

    fn close(&mut self, _: &CloseMessage, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(MessageEvent::Close);
    }
}

impl Focusable for MessageView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for MessageView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_open(window, cx);
        let t = cx.tokens().clone();
        let gone = self.gone.then(|| {
            div()
                .flex_none()
                .px(px(space::SP_3))
                .pb(px(space::SP_2))
                .child(Notice::new("message-gone", Kind::Warning, text::gone()))
        });
        div()
            .id("message-view")
            .when(self.window, |el| el.key_context(WINDOW_CONTEXT))
            .on_action(cx.listener(Self::close))
            .track_focus(&self.focus)
            .access_keys()
            .size_full()
            .flex()
            .flex_col()
            .text_color(t.color.ink)
            .child(
                Pane::new("message-pane").child(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .child(self.command_bar(cx))
                        .children(gone)
                        .child(div().flex_1().min_h_0().child(self.reader.clone())),
                ),
            )
    }
}

fn brief(db: &Db, id: i64) -> Option<komorebi::db::Brief> {
    db.briefs(&[id]).ok().and_then(|b| b.first().copied())
}

/// What the command bar shows for message `id`, read from the cache. Move
/// is on when its account has another folder; the menu's own list (the
/// recent ones first) is read only when it opens.
fn read_bar(db: &Db, id: i64) -> Bar {
    let Some(m) = brief(db, id) else {
        return Bar::default();
    };
    let folders = db.folders().unwrap_or_default();
    let role = folders
        .iter()
        .find(|f| f.id == m.folder_id)
        .map(|f| f.role)
        .unwrap_or_default();
    Bar {
        seen: m.seen,
        flagged: m.flagged,
        junk: role == FolderRole::Junk,
        // What `triage::move_options_in` would list, without its tree.
        can_move: folders.iter().any(|f| {
            f.account_id == m.account_id && f.id != m.folder_id && f.role != FolderRole::Drafts
        }),
    }
}

/// The Move menu: the message's folders, the recent ones first and then
/// all of them, a separator between (the flyout's indents are left out: a
/// menu lists them flat).
fn move_menu(
    view: WeakEntity<MessageView>,
    menu: PopupMenu,
    _window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let Ok((id, options)) = view.read_with(cx, |v, _| (v.id, v.move_options())) else {
        return menu;
    };
    let mut menu = menu::brand(menu, cx);
    let mut in_recent = options.first().is_some_and(|o| o.recent);
    for o in options {
        if in_recent && !o.recent {
            menu = menu.separator();
            in_recent = false;
        }
        let view = view.clone();
        let folder = o.id;
        menu = menu.item(PopupMenuItem::new(o.label).on_click(move |_, _, cx| {
            let _ = view.update(cx, |v, cx| {
                v.ask(
                    MessageEvent::Move {
                        message: id,
                        folder,
                    },
                    cx,
                )
            });
        }));
    }
    menu
}

/// The More menu ([`MessageView::more_entries`]).
fn more_menu(
    view: WeakEntity<MessageView>,
    menu: PopupMenu,
    _window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let Ok(groups) = view.read_with(cx, |v, cx| v.more_entries(cx)) else {
        return menu;
    };
    let mut menu = menu::brand(menu, cx);
    for group in groups.into_iter().filter(|g| !g.is_empty()) {
        menu = menu.separator();
        for e in group {
            let run = e.run;
            let view = view.clone();
            let mut item = PopupMenuItem::new(e.label).disabled(!e.enabled);
            if let Some(checked) = e.checked {
                item = item.checked(checked);
            }
            menu = menu.item(item.on_click(move |_, window, cx| {
                let _ = view.update(cx, |v, cx| v.run_more(run, window, cx));
            }));
        }
    }
    menu
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{AnyWindowHandle, TestAppContext};
    use komorebi::role::Role as FolderRole;

    use super::*;
    use crate::test_support::{self, assert_named, click_named, in_window};

    struct Rig {
        _cache: test_support::TempCache,
        db: komorebi::db::Db,
        window: AnyWindowHandle,
        view: Entity<MessageView>,
        events: Rc<RefCell<Vec<MessageEvent>>>,
        id: i64,
        seeded: komorebi::seed::Seeded,
    }

    /// A message tab's view on a seeded cache with a Junk folder.
    fn rig(name: &str, cx: &mut TestAppContext) -> Rig {
        let (cache, db, seeded) = test_support::seeded(name, 3);
        db.upsert_folder(seeded.account, "Junk", "Junk", 0, FolderRole::Junk)
            .expect("a Junk folder");
        let id = db.message_ids(&[seeded.inbox], "").expect("ids")[0];
        db.set_seen(id, false).expect("unread");
        db.set_flagged(id, false).expect("not important");
        test_support::init_app(cx);
        let view_db = Rc::new(cache.open());
        let (window, view) = test_support::open(cx, move |window, cx| {
            cx.new(|cx| MessageView::new(view_db, id, false, window, cx))
        });
        let events: Rc<RefCell<Vec<MessageEvent>>> = Rc::default();
        let sink = events.clone();
        cx.update(|cx| {
            cx.subscribe(&view, move |_, e: &MessageEvent, _| {
                sink.borrow_mut().push(e.clone())
            })
            .detach()
        });
        cx.run_until_parked();
        Rig {
            _cache: cache,
            db,
            window,
            view,
            events,
            id,
            seeded,
        }
    }

    /// The tab's command bar has the reading pane's groups: Move, Mark as
    /// read, Mark as important and More beside Reply, Archive and Delete;
    /// each asks its owner for its message.
    #[gpui_kit::test]
    fn a_message_tab_has_the_list_commands(cx: &mut TestAppContext) {
        let r = rig("tab-commands", cx);
        in_window(cx, r.window, |w, _| {
            for name in [
                "Reply",
                "Archive",
                "Delete",
                "Move",
                "Mark as read",
                "Mark as important",
                "Open in new window",
                "More actions",
            ] {
                assert_named(w, name, Role::Button);
            }
        });
        in_window(cx, r.window, |w, cx| {
            click_named(w, "Mark as important", cx);
            click_named(w, "Mark as read", cx);
        });
        cx.run_until_parked();
        assert_eq!(
            r.events.borrow()[..],
            [MessageEvent::Flag(r.id), MessageEvent::Read(r.id)]
        );
        // The bar follows the cache when told: read and important now. A
        // redraw alone reads nothing.
        r.db.set_seen(r.id, true).unwrap();
        r.db.set_flagged(r.id, true).unwrap();
        r.view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        in_window(cx, r.window, |w, _| {
            assert_named(w, "Mark as read", Role::Button)
        });
        r.view.update(cx, |v, cx| v.refresh(cx));
        cx.run_until_parked();
        in_window(cx, r.window, |w, _| {
            assert_named(w, "Mark as unread", Role::Button);
            assert_named(w, "Remove importance", Role::Button);
        });
        let bar = r.view.read_with(cx, |v, _| v.bar());
        assert!(bar.seen && bar.flagged && !bar.junk && bar.can_move);
    }

    /// Move lists the account's other folders, the recent ones first; More
    /// has Forward as attachment, Report junk (Not junk in Junk), the
    /// reader's view toggles and the source commands.
    #[gpui_kit::test]
    fn a_message_tabs_move_and_more_menus(cx: &mut TestAppContext) {
        let r = rig("tab-menus", cx);
        let ids = |v: &MessageView| v.move_options().iter().map(|o| o.id).collect::<Vec<_>>();
        let options = r.view.read_with(cx, |v, _| ids(v));
        assert!(!options.contains(&r.seeded.inbox), "never its own folder");
        assert!(options.contains(&r.seeded.archive) && options.contains(&r.seeded.sent));
        r.db.set_setting("move_recent", &r.seeded.sent.to_string())
            .unwrap();
        let first = r.view.read_with(cx, |v, _| v.move_options()[0].clone());
        assert!(first.recent && first.id == r.seeded.sent);

        let labels = r.view.read_with(cx, |v, cx| {
            v.more_entries(cx)
                .into_iter()
                .map(|g| g.into_iter().map(|e| e.label).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        });
        assert_eq!(labels[0], ["Forward as attachment"]);
        assert_eq!(labels[1], ["Report junk"]);
        assert_eq!(labels[2].len(), 3, "the reader's view toggles");
        assert_eq!(
            labels[3],
            ["Save as .eml", "Show original", "Copy Message-ID"]
        );
        in_window(cx, r.window, |w, cx| {
            r.view.update(cx, |v, cx| {
                v.run_more(More::ForwardAttached, w, cx);
                v.run_more(More::Junk, w, cx);
            })
        });
        cx.run_until_parked();
        assert_eq!(
            r.events.borrow()[..],
            [
                MessageEvent::ForwardAttached(r.id),
                MessageEvent::Junk(r.id)
            ]
        );
    }
}
