//! The formatted composer and named signatures on the real app, headless
//! (the `Harness` of `composer::tests`): what the keys and the toolbar do
//! to the document, saving and reopening, what is sent, the switch to plain
//! text, replies, the Signature menu and Settings > Signatures.

use super::*;
use crate::{EdAction, EdKey, Rich, Sigs};
use komorebi::mail::rich as model;

/// The app with new messages formatted, a new one open and its body
/// focused.
fn formatted(name: &str) -> Harness {
    let h = Harness::new(name);
    h.app(|a| a.compose_rich = true);
    h.ui().invoke_compose();
    h.frame();
    h.app(|a| a.focus_body());
    h.frame();
    h
}

fn doc(h: &Harness) -> model::Doc {
    h.app(|a| {
        a.drafts
            .active()
            .and_then(|d| d.rich.as_ref())
            .map(|e| e.doc().clone())
            .expect("a formatted draft")
    })
}

/// Paragraph `i` of the message.
fn para(h: &Harness, i: usize) -> model::Para {
    match doc(h).blocks.get(i) {
        Some(model::Block::Para(p)) => p.clone(),
        other => panic!("block {i} is {other:?}"),
    }
}

/// Type `text` a key at a time, as the keyboard does.
fn type_text(h: &Harness, text: &str) {
    for c in text.chars() {
        h.keys(&[], &c.to_string());
    }
}

/// A signature with `text`, named `name`, with a logo if `logo`.
fn add_signature(db: &Db, name: &str, text: &str, logo: bool) -> i64 {
    let logo =
        logo.then(|| model::Logo::from_bytes(&crate::app::signatures::sample_logo()).unwrap());
    db.add_signature(name, &model::Doc::from_plain(text), logo.as_ref())
        .unwrap()
}

#[test]
fn keys_type_format_and_undo_in_the_body() {
    let h = formatted("rich-keys");
    type_text(&h, "Hello");
    assert_eq!(rich_text(&h), "Hello");
    // Shift+Home selects to the start of the line (the page's Home), and
    // Ctrl+B makes it bold.
    h.keys(&[Key::Shift], &key(Key::Home));
    h.keys(&[Key::Control], "b");
    assert!(para(&h, 0).runs.iter().all(|r| r.style.b));
    assert!(h.ui().global::<Rich>().get_body_fmt().b);
    // Ctrl+E centres the paragraph here (it searches elsewhere), and
    // Ctrl+U underlines (it marks as unread elsewhere).
    h.keys(&[Key::Control], "e");
    assert_eq!(para(&h, 0).align, model::Align::Center);
    h.keys(&[Key::Control], "u");
    assert!(para(&h, 0).runs.iter().all(|r| r.style.u));
    assert!(h.ui().get_composing());
    // One undo stack for text and formatting: underline, centre, bold,
    // then the typing.
    h.keys(&[Key::Control], "z");
    assert!(!para(&h, 0).runs.iter().any(|r| r.style.u));
    h.keys(&[Key::Control], "z");
    assert_eq!(para(&h, 0).align, model::Align::Left);
    h.keys(&[Key::Control], "z");
    assert!(!para(&h, 0).runs.iter().any(|r| r.style.b));
    h.keys(&[Key::Control], "y");
    assert!(para(&h, 0).runs.iter().all(|r| r.style.b));
    // The toolbar's buttons do the same on the selection.
    h.keys(&[], &key(Key::End));
    h.keys(&[Key::Shift], &key(Key::Home));
    h.ui()
        .global::<Rich>()
        .invoke_act(rich::BODY, EdAction::Italic, 0);
    assert!(para(&h, 0).runs.iter().all(|r| r.style.i));
    h.ui()
        .global::<Rich>()
        .invoke_act(rich::BODY, EdAction::Color, 8);
    assert!(
        para(&h, 0)
            .runs
            .iter()
            .all(|r| r.style.color == Some(model::ColorId::Blue))
    );
    // The draft is saved soon after, as typing.
    assert!(h.app(|a| a.drafts.active().unwrap().dirty));
}

#[test]
fn a_list_starts_as_typed_and_undo_takes_back_only_that() {
    let h = formatted("rich-autolist");
    type_text(&h, "- ");
    let p = para(&h, 0);
    assert_eq!(p.list, Some((model::ListKind::Bullet, 0)));
    assert!(p.is_empty());
    type_text(&h, "milk");
    h.keys(&[], &key(Key::Return));
    type_text(&h, "eggs");
    assert_eq!(para(&h, 1).plain_text(), "eggs");
    assert_eq!(para(&h, 1).list, Some((model::ListKind::Bullet, 0)));
    // Tab in a list: one level in.
    h.keys(&[], &key(Key::Tab));
    assert_eq!(para(&h, 1).list, Some((model::ListKind::Bullet, 1)));
}

#[test]
fn undo_right_after_a_list_starts_puts_the_marks_back() {
    let h = formatted("rich-autolist-undo");
    type_text(&h, "1. ");
    assert_eq!(para(&h, 0).list, Some((model::ListKind::Number, 0)));
    h.keys(&[Key::Control], "z");
    assert_eq!(para(&h, 0).list, None);
    assert_eq!(rich_text(&h), "1. ");
}

#[test]
fn tables_insert_and_tab_moves_between_cells() {
    let h = formatted("rich-table");
    h.ui()
        .global::<Rich>()
        .invoke_insert_table(rich::BODY, 2, 3);
    h.frame();
    assert!(h.ui().global::<Rich>().get_body_fmt().in_table);
    type_text(&h, "a");
    h.keys(&[], &key(Key::Tab));
    type_text(&h, "b");
    let table = doc(&h)
        .blocks
        .into_iter()
        .find_map(|b| match b {
            model::Block::Table(t) => Some(t),
            _ => None,
        })
        .expect("a table");
    assert_eq!((table.row_count(), table.col_count()), (2, 3));
    assert_eq!(table.rows[0][0].plain_text(), "a");
    assert_eq!(table.rows[0][1].plain_text(), "b");
    // A row below, then the table goes; both undo.
    h.ui()
        .global::<Rich>()
        .invoke_act(rich::BODY, EdAction::RowBelow, 0);
    let rows = |h: &Harness| {
        doc(h).blocks.iter().find_map(|b| match b {
            model::Block::Table(t) => Some(t.row_count()),
            _ => None,
        })
    };
    assert_eq!(rows(&h), Some(3));
    h.ui()
        .global::<Rich>()
        .invoke_act(rich::BODY, EdAction::DeleteTable, 0);
    assert_eq!(rows(&h), None);
    h.ui()
        .global::<Rich>()
        .invoke_key(rich::BODY, EdKey::Undo, false);
    assert_eq!(rows(&h), Some(3));
    // Backspace at a cell's start never joins it with the cell before.
    let before = doc(&h);
    h.keys(&[], &key(Key::Home));
    h.keys(&[], &key(Key::Backspace));
    assert_eq!(doc(&h), before);
}

#[test]
fn a_formatted_draft_is_saved_and_opens_again_the_same() {
    let h = formatted("rich-save");
    type_text(&h, "Hi Ana");
    h.keys(&[Key::Shift], &key(Key::Home));
    h.keys(&[Key::Control], "b");
    h.app(|a| a.set_draft_field(ComposeField::To, "ana@example.org"));
    let before = doc(&h);
    let id = h.app(|a| a.drafts.active_id().unwrap());
    assert!(h.app(|a| a.save_draft(id)));
    let row = h.app(|a| a.drafts.get(id).unwrap().row.unwrap());
    let db = Db::open(&h.path).unwrap();
    let saved = db.draft(row).unwrap().unwrap();
    // The text for search and older builds, the document beside it.
    assert_eq!(saved.body, "Hi Ana");
    assert!(
        saved
            .body_rich
            .as_deref()
            .is_some_and(|h| h.contains("<strong>"))
    );
    let (back, _) = load(&db, row).unwrap().unwrap();
    assert_eq!(back.rich.as_ref().unwrap().doc(), &before);
    // Closed and opened from Drafts: formatted again.
    h.ui().invoke_c_close();
    h.frame();
    h.app(|a| a.open_saved(row));
    h.frame();
    assert!(h.ui().get_c_rich());
    assert_eq!(rich_text(&h), "Hi Ana");
}

#[test]
fn formatted_mail_goes_out_as_text_and_html_with_the_logo() {
    let h = Harness::new("rich-send");
    let db = Db::open(&h.path).unwrap();
    let account = db.accounts().unwrap().remove(0);
    let sig = add_signature(&db, "Work", "Me\nExample Org", true);
    db.set_account_signatures(account.id, Some(sig), Some(sig))
        .unwrap();
    h.app(|a| a.compose_rich = true);
    h.ui().invoke_compose();
    h.frame();
    h.app(|a| a.focus_body());
    h.frame();
    type_text(&h, "Hi");
    let out = h.app(|a| {
        let id = a.drafts.active_id().unwrap();
        let d = a.drafts.remove(id).unwrap();
        outgoing(&a.db, &account, d)
    });
    assert!(out.body.starts_with("Hi\n"), "{}", out.body);
    assert!(out.body.contains("-- \nMe\nExample Org"), "{}", out.body);
    let html = out.body_html.clone().expect("an HTML part");
    assert_eq!(out.inline.len(), 1);
    assert!(
        html.contains(&format!("cid:{}", out.inline[0].cid)),
        "{html}"
    );
    // The whole message, with the logo as a related part.
    let raw = mail::smtp::build_raw(Outgoing {
        to: "ana@example.org".into(),
        ..out
    })
    .unwrap();
    let raw = String::from_utf8_lossy(&raw);
    assert!(raw.contains("multipart/related"), "{raw}");
}
#[test]
fn a_message_with_no_formatting_goes_out_as_plain_text_only() {
    let h = formatted("rich-plain-out");
    type_text(&h, "Just text");
    let account = h.app(|a| a.accounts[0].clone());
    let out = h.app(|a| {
        let id = a.drafts.active_id().unwrap();
        let d = a.drafts.remove(id).unwrap();
        outgoing(&a.db, &account, d)
    });
    assert_eq!(out.body, "Just text");
    assert!(out.body_html.is_none() && out.inline.is_empty());
}

#[test]
fn switching_to_plain_text_can_be_undone() {
    let h = formatted("rich-switch");
    type_text(&h, "Hi");
    h.keys(&[Key::Shift], &key(Key::Home));
    h.keys(&[Key::Control], "b");
    let before = doc(&h);
    // More > Switch to plain text asks first; yes: the text, no marks.
    h.ui().global::<Rich>().set_plain_ask(true);
    h.ui().global::<Rich>().invoke_set_rich(false);
    h.frame();
    assert!(!h.ui().get_c_rich());
    assert_eq!(h.ui().get_c_body(), "Hi");
    // Ctrl+Z (the plain body's) brings the formatted message back.
    assert!(!h.ui().invoke_c_unformat().done);
    h.frame();
    assert!(h.ui().get_c_rich());
    assert_eq!(doc(&h), before);
    // Typed in plain text first, switching back starts from the text.
    h.ui().global::<Rich>().invoke_set_rich(false);
    type_into(h.ui(), ComposeField::Body, "Hi there");
    h.ui().global::<Rich>().invoke_set_rich(true);
    assert_eq!(rich_text(&h), "Hi there");
    assert!(!para(&h, 0).runs.iter().any(|r| r.style.b));
}

#[test]
fn a_formatted_reply_keeps_the_original_as_its_quote() {
    let h = Harness::new("rich-reply");
    h.app(|a| a.compose_rich = true);
    let id = h.message;
    h.app(|a| a.select_message(id));
    h.app(|a| a.reply(false));
    h.frame();
    assert!(h.ui().get_c_rich());
    // Folded behind "Show quoted text", and still sent.
    assert!(h.ui().get_c_quote_folded());
    let quote = doc(&h).blocks.iter().find_map(|b| match b {
        model::Block::Quote(q) => Some(q.clone()),
        _ => None,
    });
    let quote = quote.expect("the stored quote");
    assert!(
        quote.folded && quote.text().contains("Noon?"),
        "{}",
        quote.text()
    );
    let text = h.app(|a| a.drafts.active().unwrap().full_body());
    assert!(text.contains("Noon?"), "{text}");
    // Typing goes above it.
    h.app(|a| a.focus_body());
    h.frame();
    type_text(&h, "Ok");
    assert!(rich_text(&h).starts_with("Ok"));
    // Shown, then made ordinary quoted text to edit.
    h.ui().invoke_c_unfold_quote();
    h.frame();
    assert!(!h.ui().get_c_quote_folded());
    assert!(h.ui().global::<Rich>().get_body_fmt().quote_shown);
    h.ui()
        .global::<Rich>()
        .invoke_act(rich::BODY, EdAction::EditQuote, 0);
    let d = doc(&h);
    assert!(!d.blocks.iter().any(|b| matches!(b, model::Block::Quote(_))));
    assert!(d.blocks.iter().any(
        |b| matches!(b, model::Block::Para(p) if p.quote > 0 && p.plain_text().contains("Noon?"))
    ));
}

#[test]
fn the_signature_menu_and_another_from_account_swap_the_signature() {
    let h = Harness::new("rich-signature");
    let db = Db::open(&h.path).unwrap();
    let me = db.accounts().unwrap().remove(0);
    let a = add_signature(&db, "A", "Ana", false);
    let b = add_signature(&db, "B", "Ana Silva\nExample Co.", false);
    db.set_account_signatures(me.id, Some(a), Some(b)).unwrap();
    let source = |h: &Harness| {
        doc(h).blocks.iter().find_map(|x| match x {
            model::Block::Signature(s) => s.source,
            _ => None,
        })
    };
    // A new message gets the account's one for new messages; the menu
    // marks it.
    h.app(|x| x.compose_rich = true);
    h.ui().invoke_compose();
    h.frame();
    assert_eq!(source(&h), Some(a));
    let names: Vec<String> = h
        .ui()
        .global::<Rich>()
        .get_sig_names()
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(names, ["A", "B"]);
    assert_eq!(h.ui().global::<Rich>().get_sig_current(), 0);
    // A pick replaces it; Ctrl+Z puts the first back; None takes it out.
    h.ui().global::<Rich>().invoke_pick_signature(1);
    assert_eq!(source(&h), Some(b));
    h.ui()
        .global::<Rich>()
        .invoke_key(rich::BODY, EdKey::Undo, false);
    assert_eq!(source(&h), Some(a));
    h.ui().global::<Rich>().invoke_pick_signature(-1);
    assert_eq!(source(&h), None);
    h.app(|x| x.discard_confirmed());

    // Another From account swaps it while untouched (its default for new
    // messages), and not once edited.
    let work = db
        .add_account(&NewAccount {
            imap_user: String::new(),
            smtp_user: String::new(),
            email: "me@work.example".into(),
            name: "Me".into(),
            imap: me.imap.clone(),
            smtp: me.smtp.clone(),
            save_sent: true,
        })
        .unwrap();
    db.set_account_signatures(work, Some(b), None).unwrap();
    h.app(|x| x.load_accounts().unwrap());
    h.ui().invoke_compose();
    h.frame();
    h.ui().invoke_c_from_changed(1);
    h.frame();
    assert_eq!(source(&h), Some(b));
    h.ui().invoke_c_from_changed(0);
    h.frame();
    assert_eq!(source(&h), Some(a));
    // A reply takes the account's one for replies and forwards.
    h.app(|x| x.discard_confirmed());
    let id = h.message;
    h.app(|x| x.select_message(id));
    h.app(|x| x.reply(false));
    h.frame();
    assert_eq!(source(&h), Some(b));
}

#[test]
fn signatures_are_made_renamed_and_deleted_in_settings() {
    let h = Harness::new("rich-sig-settings");
    let ui = h.ui();
    let sigs = ui.global::<Sigs>();
    let account = h.app(|a| a.accounts[0].id);
    // Add: a name is needed.
    sigs.invoke_add();
    h.frame();
    assert!(sigs.get_editing());
    assert!(!sigs.invoke_save());
    assert_eq!(sigs.get_name_error(), "Give the signature a name.");
    sigs.set_name("Work".into());
    h.app(|a| a.focus_body());
    ui.global::<Rich>()
        .invoke_edited(rich::SIG, "Ana Silva".into(), 9, 9);
    assert!(sigs.invoke_save());
    assert!(!sigs.get_editing());
    assert_eq!(sigs.get_items().row_count(), 1);
    assert_eq!(sigs.get_items().row_data(0).unwrap().preview, "Ana Silva");
    // A second one may not take the same name (case ignored).
    sigs.invoke_add();
    sigs.set_name("work".into());
    assert!(!sigs.invoke_save());
    assert_eq!(
        sigs.get_name_error(),
        "A signature with this name already exists."
    );
    sigs.invoke_cancel();
    // The account page's boxes: None, then the names.
    ui.invoke_edit_account(account as i32);
    h.frame();
    let choices: Vec<String> = sigs.get_choices().iter().map(|s| s.to_string()).collect();
    assert_eq!(choices, ["None", "Work"]);
    sigs.invoke_set_defaults(account as i32, 1, 0);
    let db = Db::open(&h.path).unwrap();
    let work = db.signatures().unwrap()[0].id;
    assert_eq!(db.account_signatures(account).unwrap(), (Some(work), None));
    assert_eq!((sigs.get_acct_new(), sigs.get_acct_reply()), (1, 0));
    // Delete asks first, saying the account loses it; then it is gone and
    // the account has none.
    sigs.invoke_delete(0);
    assert!(ui.get_q_title().contains("Work"));
    assert!(ui.get_q_text().contains("1 account"), "{}", ui.get_q_text());
    h.app(|a| a.answer(true));
    assert_eq!(sigs.get_items().row_count(), 0);
    assert_eq!(db.account_signatures(account).unwrap(), (None, None));
}

#[test]
fn a_logo_over_200_kb_is_refused() {
    let h = Harness::new("rich-logo");
    let ui = h.ui();
    let sigs = ui.global::<Sigs>();
    sigs.invoke_add();
    let dir = std::env::temp_dir().join(format!("komorebi-logo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let big = dir.join("big.png");
    let mut bytes = crate::app::signatures::sample_logo();
    bytes.resize(model::MAX_LOGO_BYTES + 1, 0);
    std::fs::write(&big, &bytes).unwrap();
    h.app(|a| a.set_logo_file(&big));
    assert_eq!(
        sigs.get_logo_error(),
        "The logo must be a PNG or JPEG file of 200 KB or less."
    );
    assert!(!sigs.get_has_logo());
    let ok = dir.join("ok.png");
    std::fs::write(&ok, crate::app::signatures::sample_logo()).unwrap();
    h.app(|a| a.set_logo_file(&ok));
    assert!(sigs.get_has_logo() && sigs.get_logo_error().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

/// Typing latency on the real window: a formatted draft of 2,000 lines
/// with 20 tables, the caret in its middle; each key is dispatched to the
/// window and one frame drawn (the proxy, the model, the changed block's
/// layout, the pieces in view and Slint's software renderer). Run with
/// `cargo test --release --bin komorebi -- --ignored --nocapture typing_latency`.
#[test]
#[ignore = "a measurement (release build)"]
fn typing_latency_on_a_long_message() {
    let h = formatted("rich-latency");
    let mut blocks = Vec::new();
    let words = "The quick brown fox jumps over the lazy dog and keeps running";
    for i in 0..2000 {
        blocks.push(model::Block::Para(model::Para::text(&format!(
            "{i}: {words}"
        ))));
        if i % 100 == 50 {
            let mut t = model::Table::new(4, 4).unwrap();
            for (r, row) in t.rows.iter_mut().enumerate() {
                for (c, cell) in row.iter_mut().enumerate() {
                    *cell = model::Cell::text(&format!("r{r} c{c}"));
                }
            }
            blocks.push(model::Block::Table(t));
        }
    }
    let doc = model::Doc {
        blocks,
        ..model::Doc::default()
    };
    h.app(|a| {
        a.drafts.active_mut().unwrap().rich = Some(model::edit::Editor::new(doc));
        a.ed_reset(rich::BODY);
        let mid = a
            .drafts
            .active()
            .unwrap()
            .rich
            .as_ref()
            .unwrap()
            .text()
            .len()
            / 2;
        let mid = a.drafts.active().unwrap().rich.as_ref().unwrap().text()[..mid]
            .rfind('\n')
            .map_or(0, |i| i + 5);
        a.ed_set_caret(rich::BODY, mid);
        a.ed_show(rich::BODY, true, true, true);
    });
    h.frame();
    let draw = || {
        h.window.request_redraw();
        h.window.draw_if_needed(|r| {
            let size = h.window.size();
            let (w, hh) = (size.width as usize, size.height as usize);
            let mut buf = vec![Rgb8Pixel::default(); w * hh];
            r.render(&mut buf, w);
        });
    };
    let mut runs = Vec::new();
    let mut parts: Vec<(f64, f64)> = Vec::new();
    let runs_n: usize = std::env::var("KOMOREBI_LATENCY_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(7);
    for _ in 0..runs_n {
        let mut times = Vec::new();
        for k in 0..100 {
            let c = if k % 10 == 9 { " " } else { "x" };
            let t0 = std::time::Instant::now();
            let send = |e| h.ui().window().dispatch_event(e);
            send(WindowEvent::KeyPressed { text: c.into() });
            send(WindowEvent::KeyReleased { text: c.into() });
            slint::platform::update_timers_and_animations();
            let t1 = std::time::Instant::now();
            draw();
            times.push(t0.elapsed().as_secs_f64() * 1000.0);
            parts.push((
                t1.duration_since(t0).as_secs_f64() * 1000.0,
                t1.elapsed().as_secs_f64() * 1000.0,
            ));
        }
        times.sort_by(f64::total_cmp);
        runs.push((times[times.len() / 2], times[times.len() * 95 / 100]));
    }
    let mut med: Vec<f64> = runs.iter().map(|r| r.0).collect();
    let mut p95: Vec<f64> = runs.iter().map(|r| r.1).collect();
    med.sort_by(f64::total_cmp);
    p95.sort_by(f64::total_cmp);
    println!(
        "typing latency (key to drawn frame, 1280x800): median {:.2} ms (min {:.2}), p95 {:.2} ms (min {:.2}) over {} runs of 100 keys",
        med[med.len() / 2],
        med[0],
        p95[p95.len() / 2],
        p95[0],
        runs.len()
    );
    let mut keys: Vec<f64> = parts.iter().map(|p| p.0).collect();
    let mut draws: Vec<f64> = parts.iter().map(|p| p.1).collect();
    keys.sort_by(f64::total_cmp);
    draws.sort_by(f64::total_cmp);
    println!(
        "  of which the key's handling (proxy, model, layout, pieces): median {:.2} ms; the frame's drawing: median {:.2} ms",
        keys[keys.len() / 2],
        draws[draws.len() / 2]
    );
    let text = rich_text(&h);
    let at = text.find('x').unwrap_or(0);
    assert!(
        text.contains("xxxxxxxxx "),
        "typed nothing? {} bytes, near: {:?}",
        text.len(),
        text.get(at.saturating_sub(20)..(at + 40).min(text.len()))
    );
}

#[test]
fn typing_far_down_a_long_message_reaches_the_caret() {
    // The hidden input once took keys only on its first lines.
    let h = formatted("rich-far");
    let blocks: Vec<model::Block> = (0..300)
        .map(|i| model::Block::Para(model::Para::text(&format!("{i}: words here"))))
        .collect();
    let at = h.app(|a| {
        a.drafts.active_mut().unwrap().rich = Some(model::edit::Editor::new(model::Doc {
            blocks,
            ..model::Doc::default()
        }));
        a.ed_reset(rich::BODY);
        let ed = a.drafts.active().unwrap().rich.as_ref().unwrap();
        let at = ed.text().find("250: ").unwrap() + 5;
        a.ed_set_caret(rich::BODY, at);
        a.ed_show(rich::BODY, true, true, true);
        at
    });
    h.frame();
    type_text(&h, "Z");
    assert_eq!(rich_text(&h).get(at..at + 6), Some("Zwords"));
    // Down goes to the next line at the same x.
    h.keys(&[], &key(Key::DownArrow));
    type_text(&h, "Y");
    let text = rich_text(&h);
    let line = text.lines().find(|l| l.starts_with("251: ")).unwrap();
    assert!(
        line.contains('Y') && line.len() == "251: words here".len() + 1,
        "{line}"
    );
}

#[test]
fn a_message_longer_than_the_proxy_window_types_everywhere() {
    let h = formatted("rich-window");
    let blocks: Vec<model::Block> = (0..2000)
        .map(|i| model::Block::Para(model::Para::text(&format!("{i}: words here"))))
        .collect();
    let at = h.app(|a| {
        a.drafts.active_mut().unwrap().rich = Some(model::edit::Editor::new(model::Doc {
            blocks,
            ..model::Doc::default()
        }));
        a.ed_reset(rich::BODY);
        let ed = a.drafts.active().unwrap().rich.as_ref().unwrap();
        assert!(ed.text().len() > rich::WINDOW);
        let at = ed.text().find("1500: ").unwrap() + 6;
        a.ed_set_caret(rich::BODY, at);
        a.ed_show(rich::BODY, true, true, true);
        at
    });
    h.frame();
    type_text(&h, "Z");
    assert_eq!(rich_text(&h).get(at..at + 6), Some("Zwords"));
    // Right through the window's edge, a character at a time.
    // (Past the window's edge: half a window is about 500 of these lines.)
    for _ in 0..420 {
        h.keys(&[], &key(Key::DownArrow));
    }
    type_text(&h, "Q");
    assert!(
        rich_text(&h)
            .lines()
            .any(|l| l.starts_with("1920: ") && l.contains('Q'))
    );
    // Ctrl+End and Ctrl+Home reach the message's ends.
    h.keys(&[Key::Control], &key(Key::End));
    type_text(&h, "E");
    assert!(rich_text(&h).ends_with("1999: words hereE"));
    h.keys(&[Key::Control], &key(Key::Home));
    type_text(&h, "S");
    assert!(rich_text(&h).starts_with("S0: words here"));
    // Ctrl+A selects all of it: typing replaces the message.
    h.keys(&[Key::Control], "a");
    type_text(&h, "x");
    assert_eq!(rich_text(&h), "x");
}

#[test]
fn a_message_switched_to_plain_text_goes_out_as_plain_text_only() {
    let h = formatted("rich-plain-only");
    type_text(&h, "- milk");
    h.keys(&[], &key(Key::Return));
    type_text(&h, "**eggs**");
    assert_eq!(para(&h, 0).list, Some((model::ListKind::Bullet, 0)));
    h.ui().global::<Rich>().invoke_set_rich(false);
    h.frame();
    let account = h.app(|a| a.accounts[0].clone());
    let out = h.app(|a| {
        let id = a.drafts.active_id().unwrap();
        let d = a.drafts.remove(id).unwrap();
        outgoing(&a.db, &account, d)
    });
    // The list's marks and the asterisks stay text; nothing else goes.
    assert!(
        out.body.contains("- milk") && out.body.contains("**eggs**"),
        "{}",
        out.body
    );
    assert!(out.body_html.is_none() && out.inline.is_empty());
}

#[test]
fn a_formatted_reply_does_not_ask_about_an_attachment_the_original_mentions() {
    let h = Harness::new("rich-guard");
    let mut db = Db::open(&h.path).unwrap();
    let raw = b"From: Bob <bob@example.org>\r\nTo: me@example.org\r\n\
        Subject: Lunch\r\nMessage-ID: <m1@example.org>\r\n\r\nSee the attached menu.\r\n";
    db.set_body(
        h.message,
        raw,
        "See the attached menu.",
        false,
        Default::default(),
    )
    .unwrap();
    h.app(|a| a.compose_rich = true);
    let id = h.message;
    h.app(|a| a.select_message(id));
    h.app(|a| a.reply(false));
    h.frame();
    assert!(h.ui().get_c_rich());
    h.app(|a| a.set_draft_field(ComposeField::Body, "Sounds good"));
    assert_eq!(h.app(|a| send_guard(a.drafts.active().unwrap())), None);
    // Its own text still counts.
    h.app(|a| a.set_draft_field(ComposeField::Body, "Attached is mine. "));
    assert_eq!(
        h.app(|a| send_guard(a.drafts.active().unwrap())),
        Some(Guard::NoAttachment)
    );
}
