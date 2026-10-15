//! Named signatures (docs/WINUI3.md "Signatures page"): Settings >
//! Signatures (the list, and one signature's page with its own formatted
//! editor and at most one logo), each account's two defaults (new messages;
//! replies and forwards) on its page, and the composer's Signature menu.
//! Also the composer's switch between formatted and plain text, and the
//! defaults new messages start with (Settings > General > Compose).

use super::composer::rich::{self, SIG};
use super::*;
use crate::{Rich, SigRow, Sigs};
use komorebi::mail::render::canvas;
use komorebi::mail::rich::edit::Editor;
use komorebi::mail::rich::{self as model, ColorId, FontId, Logo, LogoError, SizePt};

/// The composer's Signature menu: "Manage signatures…".
const MANAGE: i32 = -2;

impl App {
    /// Read the signatures again, for the list, the menus and the boxes.
    pub(super) fn reload_signatures(&mut self) {
        match self.db.signatures() {
            Ok(list) => self.sig_list = list,
            Err(e) => self.set_status(&text::detail(&e), true),
        }
        let ui = self.ui();
        let rows: Vec<SigRow> = self
            .sig_list
            .iter()
            .map(|s| SigRow {
                name: s.name.as_str().into(),
                preview: s.first_line.as_str().into(),
                logo: s.has_logo,
            })
            .collect();
        let sigs = ui.global::<Sigs>();
        sigs.set_items(ModelRc::new(VecModel::from(rows)));
        let none = SharedString::from(t!(invoke_sig_none));
        let choices: Vec<SharedString> = std::iter::once(none)
            .chain(self.sig_list.iter().map(|s| s.name.as_str().into()))
            .collect();
        sigs.set_choices(ModelRc::new(VecModel::from(choices)));
        let names: Vec<SharedString> = self
            .sig_list
            .iter()
            .map(|s| s.name.as_str().into())
            .collect();
        ui.global::<Rich>()
            .set_sig_names(ModelRc::new(VecModel::from(names)));
    }

    /// Where signature `id` is in the list (-1: none or gone).
    fn sig_index(&self, id: Option<i64>) -> i32 {
        id.and_then(|id| self.sig_list.iter().position(|s| s.id == id))
            .map_or(-1, |i| i as i32)
    }

    /// The composer's Signature menu, for the draft shown.
    pub(super) fn show_signature_choices(&mut self) {
        self.reload_signatures();
        let current = match self.drafts.active() {
            Some(d) if d.rich.is_some() => {
                self.sig_index(d.rich_signature().and_then(|s| s.source))
            }
            Some(d) => {
                // A plain draft: the signature whose block it holds.
                let block = d.signature.as_deref().unwrap_or_default();
                match block {
                    "" => -1,
                    b => self
                        .sig_list
                        .iter()
                        .position(|s| {
                            self.db
                                .signature(s.id)
                                .ok()
                                .flatten()
                                .is_some_and(|x| composer::signature_block(&x.body_text) == b)
                        })
                        .map_or(-1, |i| i as i32),
                }
            }
            None => -1,
        };
        self.ui().global::<Rich>().set_sig_current(current);
    }

    /// A pick from the composer's Signature menu: it replaces the
    /// signature in the message, even an edited one (Ctrl+Z puts it back
    /// in a formatted message).
    pub(super) fn pick_signature(&mut self, index: i32) {
        if index == MANAGE {
            let ui = self.ui();
            ui.set_settings_section(1);
            ui.set_show_settings(true);
            return;
        }
        let sig = usize::try_from(index)
            .ok()
            .and_then(|i| self.sig_list.get(i))
            .and_then(|s| self.db.signature(s.id).ok().flatten());
        let Some(d) = self.drafts.active_mut() else {
            return;
        };
        if let Some(ed) = d.rich.as_mut() {
            if ed.set_signature(sig.as_ref().and_then(db::Signature::block)) {
                d.touched = true;
                d.dirty = true;
                self.ed_show(rich::BODY, true, true, false);
                self.schedule_autosave();
            }
        } else {
            let new = sig
                .as_ref()
                .map_or_else(String::new, |s| composer::signature_block(&s.body_text));
            d.replace_signature(&new);
            let body = SharedString::from(d.body.as_str());
            self.ui().set_c_body(body);
            self.schedule_autosave();
        }
        self.ui().global::<Rich>().set_sig_current(index.max(-1));
    }

    /// The message's format from its More menu: plain text (asked first)
    /// or formatted again.
    pub(super) fn set_rich(&mut self, on: bool) {
        let defaults = self.compose_defaults;
        let Some(d) = self.drafts.active_mut() else {
            return;
        };
        if on {
            if d.rich.is_some() {
                return;
            }
            let ed = match d.unplain.take() {
                // Nothing typed since the switch: the document as it was.
                Some((ed, text)) if text == d.full_body() => ed,
                _ => Editor::new(rich::doc_from_text(&d.full_body(), defaults)),
            };
            d.rich = Some(ed);
            d.body.clear();
            d.quote = None;
            d.quoted_html = None;
            d.signature = None;
        } else {
            let Some(ed) = d.rich.take() else {
                return;
            };
            let text = model::plain(ed.doc());
            d.body = text.clone();
            d.unplain = Some((ed, text));
            d.signature = None;
            d.quote = None;
            d.quoted_html = None;
        }
        d.touched = true;
        d.dirty = true;
        let (formatted, body) = (d.rich.is_some(), SharedString::from(d.body.as_str()));
        let ui = self.ui();
        ui.set_c_body(body);
        ui.set_c_rich(formatted);
        ui.set_c_quote_folded(false);
        self.ed_reset(rich::BODY);
        if formatted {
            self.ed_show(rich::BODY, true, true, false);
        }
        self.focus_body();
        self.schedule_autosave();
    }

    /// Settings > General > Compose changed.
    pub(super) fn compose_defaults_changed(
        &mut self,
        format: i32,
        font: i32,
        size: i32,
        color: i32,
    ) {
        let font = usize::try_from(font)
            .ok()
            .and_then(|i| FontId::ALL.get(i).copied())
            .unwrap_or_default();
        let size = usize::try_from(size)
            .ok()
            .and_then(|i| SizePt::ALL.get(i).copied())
            .and_then(SizePt::new)
            .unwrap_or(SizePt::DEFAULT);
        let color = usize::try_from(color)
            .ok()
            .and_then(|i| ColorId::ALL.get(i).copied());
        self.compose_rich = format != 1;
        self.compose_defaults = (font, size, color);
        let writes = [
            (
                "compose_format",
                if self.compose_rich { "rich" } else { "plain" }.to_string(),
            ),
            ("compose_font", font.label().to_string()),
            ("compose_size", size.get().to_string()),
            (
                "compose_color",
                color.map_or_else(|| "auto".into(), |c| c.hex().to_string()),
            ),
        ];
        for (key, value) in writes {
            if let Err(e) = self.db.set_setting(key, &value) {
                return self.set_status(&text::detail(&e), true);
            }
        }
        self.show_compose_defaults();
    }

    /// The Compose cards' boxes, from the settings.
    pub(super) fn show_compose_defaults(&self) {
        let (font, size, color) = self.compose_defaults;
        let index = |i: Option<usize>| i.map_or(-1, |i| i as i32);
        let ui = self.ui();
        let rich = ui.global::<Rich>();
        rich.set_def_format(i32::from(!self.compose_rich));
        rich.set_def_font(index(FontId::ALL.iter().position(|f| *f == font)).max(0));
        rich.set_def_size(index(SizePt::ALL.iter().position(|s| *s == size.get())).max(0));
        rich.set_def_color(index(
            color.and_then(|c| ColorId::ALL.iter().position(|x| *x == c)),
        ));
    }

    // ---- Settings > Signatures

    /// The account page's two boxes, for account `id`.
    pub(super) fn show_account_signatures(&mut self, id: i64) {
        self.reload_signatures();
        let (new, reply) = self.db.account_signatures(id).unwrap_or_default();
        let ui = self.ui();
        let sigs = ui.global::<Sigs>();
        // 0 is None in the boxes.
        sigs.set_acct_new(self.sig_index(new) + 1);
        sigs.set_acct_reply(self.sig_index(reply) + 1);
    }

    /// An account's boxes changed: saved at once.
    fn set_account_defaults(&mut self, account: i64, new: i32, reply: i32) {
        let id = |i: i32| {
            usize::try_from(i - 1)
                .ok()
                .and_then(|i| self.sig_list.get(i))
                .map(|s| s.id)
        };
        if let Err(e) = self.db.set_account_signatures(account, id(new), id(reply)) {
            self.set_status(&text::detail(&e), true);
        }
        self.show_account_signatures(account);
    }

    /// Open a signature's page: `id` (none: a new one).
    pub(super) fn open_signature(&mut self, id: Option<i64>) {
        let sig = id.and_then(|id| self.db.signature(id).ok().flatten());
        let (name, doc, logo) = match &sig {
            Some(s) => (s.name.clone(), s.doc(), s.logo()),
            None => (String::new(), model::Doc::default(), None),
        };
        self.sig_open = sig.map(|s| s.id);
        self.sig_logo = logo;
        self.sig_dirty = false;
        self.ed.sig = Some(Editor::new(doc));
        let ui = self.ui();
        let sigs = ui.global::<Sigs>();
        sigs.set_name(name.into());
        sigs.set_name_error(SharedString::new());
        sigs.set_logo_error(SharedString::new());
        self.show_logo();
        self.ed_reset(SIG);
        self.ed_show(SIG, true, true, false);
        ui.global::<Sigs>().set_editing(true);
    }

    /// The logo on the signature's page, as it is sent.
    fn show_logo(&self) {
        let ui = self.ui();
        let sigs = ui.global::<Sigs>();
        let shown = self.sig_logo.as_ref().and_then(|l| {
            let scale = self.ui().window().scale_factor();
            let w = (f32::from(l.width) * scale).round().max(1.0) as u32;
            let pm = canvas::decode_image(&l.bytes, w)?;
            let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                pm.data(),
                pm.width(),
                pm.height(),
            );
            Some((
                slint::Image::from_rgba8_premultiplied(buf),
                l.width,
                l.height,
            ))
        });
        match shown {
            Some((img, w, h)) => {
                sigs.set_logo(img);
                sigs.set_logo_w(f32::from(w));
                sigs.set_logo_h(f32::from(h));
                sigs.set_has_logo(true);
            }
            None => {
                sigs.set_logo(slint::Image::default());
                sigs.set_has_logo(false);
            }
        }
    }

    /// Save: the name is checked (not empty, not another signature's).
    fn save_signature(&mut self) -> bool {
        let ui = self.ui();
        let sigs = ui.global::<Sigs>();
        let name = sigs.get_name().to_string();
        let Some(doc) = self.ed.sig.as_ref().map(|e| e.doc().clone()) else {
            return false;
        };
        let saved = match self.sig_open {
            Some(id) => self
                .db
                .update_signature(id, &name, &doc, self.sig_logo.as_ref())
                .map(|()| id),
            None => self.db.add_signature(&name, &doc, self.sig_logo.as_ref()),
        };
        match saved {
            Ok(_) => {
                self.close_signature();
                true
            }
            Err(e) => {
                let said = match e.downcast_ref::<db::SignatureError>() {
                    Some(db::SignatureError::EmptyName) => t!(invoke_sig_name_empty),
                    Some(db::SignatureError::NameTaken) => t!(invoke_sig_name_taken),
                    _ => text::detail(&e),
                };
                sigs.set_name_error(said.into());
                false
            }
        }
    }

    /// Back to the list (Save or Cancel).
    fn close_signature(&mut self) {
        self.ed.sig = None;
        self.sig_open = None;
        self.sig_logo = None;
        self.sig_dirty = false;
        self.ed_reset(SIG);
        self.reload_signatures();
        self.ui().global::<Sigs>().set_editing(false);
    }

    /// The logo picked in the file dialog: checked (PNG or JPEG, 200 KB at
    /// most) and kept as it is.
    pub(super) fn set_logo_file(&mut self, path: &Path) {
        let ui = self.ui();
        let sigs = ui.global::<Sigs>();
        let read = std::fs::metadata(path)
            .ok()
            .filter(|m| m.len() <= model::MAX_LOGO_BYTES as u64)
            .and_then(|_| std::fs::read(path).ok());
        match read
            .ok_or(LogoError::TooLarge)
            .and_then(|b| Logo::from_bytes(&b))
        {
            Ok(logo) => {
                self.sig_logo = Some(logo);
                self.sig_dirty = true;
                sigs.set_logo_error(SharedString::new());
                self.show_logo();
            }
            Err(_) => sigs.set_logo_error(t!(invoke_sig_logo_refused).into()),
        }
    }

    /// Delete asks first, saying which accounts lose it as a default.
    pub(super) fn signature_question(&self, id: i64) -> (String, String, String) {
        let name = self
            .sig_list
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let users = self.db.signature_users(id).map_or(0, |u| u.len());
        let mut text = t!(invoke_q_sig_delete_text);
        if users > 0 {
            text.push(' ');
            text.push_str(&t!(invoke_q_sig_delete_users, users as i32));
        }
        (
            t!(invoke_q_sig_delete_title, name.into()),
            text,
            t!(invoke_q_delete_ok),
        )
    }

    pub(super) fn delete_signature_now(&mut self, id: i64) {
        if let Err(e) = self.db.delete_signature(id) {
            self.set_status(&text::detail(&e), true);
        }
        self.reload_signatures();
    }
}

/// A made-up 160 x 48 logo for the snapshots: three leaves of the mark's
/// blues on white, as a PNG.
pub(super) fn sample_logo() -> Vec<u8> {
    let (w, h) = (160u32, 48u32);
    let mut px = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            let band = x / 16;
            let inside = band < 3 && (y as i32 - 24).abs() < 18 - (x % 16) as i32 / 2;
            let rgb = match (inside, band) {
                (true, 0) => [0x00, 0x5a, 0x9e],
                (true, 1) => [0x00, 0x78, 0xd7],
                (true, _) => [0x42, 0x9c, 0xe3],
                _ => [0xff, 0xff, 0xff],
            };
            px.extend_from_slice(&rgb);
        }
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    if let Ok(mut writer) = enc.write_header() {
        writer.write_image_data(&px).ok();
    }
    out
}

/// The made-up message of the snapshots' formatted composer.
fn sample_doc(table: bool) -> model::Doc {
    use model::{Block, Para, Run, Style};
    let styled = |text: &str, f: &dyn Fn(&mut Style)| {
        let mut style = Style::default();
        f(&mut style);
        Run::styled(text, style)
    };
    let para = |runs: Vec<Run>| Para {
        runs,
        ..Para::default()
    };
    let item = |text: &str, kind: model::ListKind| Para {
        list: Some((kind, 0)),
        ..Para::text(text)
    };
    let mut blocks = vec![
        Block::Para(Para::text("Hi Ana,")),
        Block::Para(Para::default()),
        Block::Para(para(vec![
            Run::new("The "),
            styled("quarterly report", &|s| s.b = true),
            Run::new(" is "),
            styled("ready", &|s| s.i = true),
            Run::new(", and the "),
            styled("totals", &|s| {
                s.u = true;
                s.color = Some(ColorId::Blue);
            }),
            Run::new(" are in "),
            styled("the shared sheet", &|s| {
                s.link = Some("https://example.org/sheet".into())
            }),
            Run::new("."),
        ])),
        Block::Para(item(
            "Totals checked against March",
            model::ListKind::Bullet,
        )),
        Block::Para(item("Travel costs added", model::ListKind::Bullet)),
        Block::Para(item("Send it to the board", model::ListKind::Number)),
        Block::Para(item("Book the review", model::ListKind::Number)),
        Block::Para(Para {
            quote: 1,
            ..Para::text("Can you send it today?")
        }),
    ];
    if table {
        let mut t = model::Table::new(3, 3).unwrap_or_default();
        let cells = [
            ["Quarter", "Budget", "Spent"],
            ["Q1", "12,000", "11,400"],
            ["Q2", "14,000", "9,850"],
        ];
        for (r, row) in t.rows.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                if let Some(text) = cells.get(r).and_then(|x| x.get(c)) {
                    *cell = model::Cell::text(text);
                }
            }
        }
        blocks.push(Block::Table(t));
    }
    blocks.push(Block::Para(Para::text("Thanks,")));
    model::Doc {
        blocks,
        ..model::Doc::default()
    }
}

impl App {
    pub(super) fn sample_rich(&mut self, table: bool) {
        for (field, text) in [
            (ComposeField::To, "Ana Silva <ana@example.org>"),
            (ComposeField::Subject, "Quarterly report"),
        ] {
            self.set_draft_field(field, text);
        }
        let Some(d) = self.drafts.active_mut() else {
            return;
        };
        let Some(ed) = d.rich.as_mut() else {
            return;
        };
        if let Some(caret) = ed.paste(0, 0, sample_doc(table)) {
            d.touched = true;
            d.dirty = true;
            self.ed_set_caret(rich::BODY, caret);
        }
        self.ed_show(rich::BODY, true, true, false);
        self.focus_body();
    }

    pub(super) fn sample_signatures(&mut self, edit: bool) {
        if komorebi::dev_var("KOMOREBI_OFFLINE").is_some()
            && self.db.signatures().is_ok_and(|s| s.is_empty())
        {
            let work = rich::doc_from_text(
                "Ana Silva\nProject lead, Example Co.\n+351 210 000 000",
                (FontId::default(), SizePt::DEFAULT, None),
            );
            let short = rich::doc_from_text("Ana", (FontId::default(), SizePt::DEFAULT, None));
            let logo = Logo::from_bytes(&sample_logo()).ok();
            let first = self.db.add_signature("Work", &work, logo.as_ref()).ok();
            self.db.add_signature("Short", &short, None).ok();
            if let Some(a) = self.accounts.first() {
                self.db.set_account_signatures(a.id, first, first).ok();
            }
        }
        let ui = self.ui();
        ui.set_settings_section(1);
        ui.set_show_settings(true);
        self.reload_signatures();
        if edit
            && let Some(id) = self
                .sig_list
                .iter()
                .find(|s| s.name == "Work")
                .or(self.sig_list.first())
                .map(|s| s.id)
        {
            self.open_signature(Some(id));
        }
    }
}

/// Settings > Signatures > Add logo…: the file dialog (no app borrow held
/// across its message loop).
fn pick_logo() {
    let Some(path) = rfd::FileDialog::new()
        .set_title(t!(invoke_dlg_pick_logo))
        .add_filter("PNG, JPEG", &["png", "jpg", "jpeg"])
        .pick_file()
    else {
        return;
    };
    with_app(|a| a.set_logo_file(&path));
}

pub(super) fn wire(ui: &AppWindow) {
    let sigs = ui.global::<Sigs>();
    sigs.on_add(|| {
        with_app(|a| a.open_signature(None));
    });
    sigs.on_edit(|i| {
        with_app(|a| {
            let id = usize::try_from(i)
                .ok()
                .and_then(|i| a.sig_list.get(i))
                .map(|s| s.id);
            if id.is_some() {
                a.open_signature(id);
            }
        });
    });
    sigs.on_delete(|i| {
        with_app(|a| {
            if let Some(id) = usize::try_from(i)
                .ok()
                .and_then(|i| a.sig_list.get(i))
                .map(|s| s.id)
            {
                a.ask(Question::DeleteSignature(id));
            }
        });
    });
    sigs.on_save(|| with_app(|a| a.save_signature()).unwrap_or(false));
    sigs.on_cancel(|| {
        with_app(|a| a.close_signature());
    });
    sigs.on_add_logo(pick_logo);
    sigs.on_remove_logo(|| {
        with_app(|a| {
            a.sig_logo = None;
            a.sig_dirty = true;
            a.show_logo();
        });
    });
    sigs.on_set_defaults(|account, new, reply| {
        with_app(|a| a.set_account_defaults(i64::from(account), new, reply));
    });
    let rich = ui.global::<Rich>();
    rich.on_pick_signature(|i| {
        with_app(|a| a.pick_signature(i));
    });
    rich.on_set_rich(|on| {
        with_app(|a| a.set_rich(on));
    });
    rich.on_defaults_changed(|format, font, size, color| {
        with_app(|a| a.compose_defaults_changed(format, font, size, color));
    });
}
