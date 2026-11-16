//! The composer model's tests (moved from the Slint app's
//! `src/app/composer.rs` with the code; docs/GPUI-TEST-PORT.md rows 53 to
//! 73 and 85 to 90).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::send::{PREVIEW_MAX_ROWS, PREVIEW_STACK, preview_page};
use super::*;
use crate::db::NewAccount;
use crate::providers::{Security, Server};

fn message(from: &str, to: &str, cc: &str) -> db::Message {
    db::Message {
        id: 1,
        account_id: 1,
        folder_id: 1,
        uid: 1,
        msgid: String::new(),
        subject: String::new(),
        from_name: String::new(),
        from_addr: from.into(),
        to: to.into(),
        cc: cc.into(),
        date: 0,
        seen: true,
        flagged: false,
        importance: db::Importance::Normal,
        raw: None,
    }
}

#[test]
fn drafts_per_kind() {
    let new = Draft::new(7);
    assert_eq!((new.account_id, new.kind), (7, Kind::New));
    assert!(new.is_empty() && !new.keep());
    assert_eq!(new.send_block(), Some(SendBlock::NoRecipient));

    let mut m = message("bob@x.org", "al@x.org, val@x.org", "cy@x.org");
    m.id = 42;
    m.account_id = 3;
    m.subject = "Lunch".into();
    m.msgid = "m1@x.org".into();
    let body = Body {
        text: "Noon?\n> earlier".into(),
        references: "<m0@x.org> <m1@x.org>".into(),
        ..Default::default()
    };
    let reply = Draft::reply(&m, &body, "al@x.org", false, &Heads::en());
    assert_eq!(reply.kind, Kind::Reply);
    assert_eq!(reply.account_id, 3);
    assert_eq!((reply.to.as_str(), reply.cc.as_str()), ("bob@x.org", ""));
    assert!(!reply.show_cc);
    assert_eq!(reply.subject, "Re: Lunch");
    assert_eq!(reply.reply_to, Some(42));
    assert_eq!(reply.in_reply_to, "m1@x.org");
    assert_eq!(reply.references, body.references);
    assert!(reply.body.starts_with("\n\nOn "), "{}", reply.body);
    assert!(
        reply.body.ends_with("> Noon?\n>> earlier"),
        "{}",
        reply.body
    );
    // Filled in by the app, not by the user: nothing worth keeping.
    assert!(!reply.keep());

    // Reply all reuses reply_recipients: me left out, Cc shown.
    let all = Draft::reply(&m, &body, "al@x.org", true, &Heads::en());
    assert_eq!(all.kind, Kind::ReplyAll);
    assert_eq!(all.to, "bob@x.org, val@x.org");
    assert_eq!(all.cc, "cy@x.org");
    assert!(all.show_cc);
    let (to, cc) = reply_recipients(&m, "", "al@x.org", true);
    assert_eq!((all.to, all.cc), (to.join(", "), cc.join(", ")));

    let file = OutAttachment {
        filename: "a.pdf".into(),
        data: vec![1, 2],
    };
    let fwd = Draft::forward(&m, &body, vec![file.into()], &Heads::en());
    assert_eq!(fwd.kind, Kind::Forward);
    assert_eq!(fwd.subject, "Fwd: Lunch");
    assert!(fwd.to.is_empty() && fwd.in_reply_to.is_empty() && fwd.reply_to.is_none());
    assert_eq!(fwd.attachments.len(), 1);
    assert!(
        fwd.body
            .contains("---------- Forwarded message ----------\nFrom:  <bob@x.org>")
    );
    assert!(fwd.body.ends_with("Noon?\n> earlier"));
}

#[test]
fn addresses_are_checked_on_leaving_and_rechecked_while_wrong() {
    let mut d = Draft::new(1);
    d.edit(Field::To, "João Silva <j@x.pt>");
    // Typing alone shows nothing; leaving the field checks it.
    d.validate(Field::To);
    assert_eq!(d.errors, [None, None, None]);
    assert_eq!(d.send_block(), None);

    d.edit(Field::Cc, "ana@x.pt, j@");
    assert!(d.errors[1].is_none());
    d.validate(Field::Cc);
    // The address at fault; the error words it in the language shown.
    assert_eq!(d.errors[1].as_deref(), Some("j@"));
    assert_eq!(d.error(Field::Cc), Some("j@"));
    assert_eq!(d.send_block(), Some(SendBlock::FixAddress(Field::Cc)));
    // Once shown, the error follows the typing and clears when right.
    d.edit(Field::Cc, "ana@x.pt, j@x");
    assert_eq!(d.errors[1], None);
    assert_eq!(d.send_block(), None);

    assert!(address_error("João Silva j@x.pt").is_some());
    assert!(address_error("\"Silva, João\" <j@x.pt>").is_none());
    // Subject and body are never checked.
    d.edit(Field::Subject, "<<<");
    d.validate(Field::Subject);
    assert_eq!(d.send_block(), None);
}

#[test]
fn attachment_mentions_in_english_and_portuguese() {
    for text in [
        "See attached.",
        "I attach the invoice",
        "The attachment is the signed copy",
        "Attachments: two",
        "Segue em anexo o relatório.",
        "Envio o documento anexado",
        "ANEXO: contrato",
        "(attached)",
    ] {
        assert!(mentions_attachment(text), "{text}");
    }
    for text in [
        "",
        "Nothing to see here",
        "The detached house",
        "reattach later",
        // Only the user's own text counts: not quoted lines...
        "Thanks!\n\nOn Monday, Bob wrote:\n> see attached\n>> anexo",
        // ...nor a forwarded message.
        "FYI\n\n---------- Forwarded message ----------\nSee attached.",
        // ...nor the signature.
        "Hi\n\n-- \nAna\nAnexos: ver site",
    ] {
        assert!(!mentions_attachment(text), "{text}");
    }
}

#[test]
fn send_guards_ask_about_subject_and_attachments() {
    let mut d = Draft::new(1);
    d.edit(Field::To, "ana@x.org");
    assert_eq!(send_guard(&d), Some(Guard::NoSubject));
    d.edit(Field::Body, "The file is attached.");
    assert_eq!(send_guard(&d), Some(Guard::Both));
    d.edit(Field::Subject, "  Report ");
    assert_eq!(send_guard(&d), Some(Guard::NoAttachment));
    assert_eq!(d.check_send(false), SendCheck::Ask(Guard::NoAttachment));
    assert_eq!(d.check_send(true), SendCheck::Ready);
    d.attachments.push(Attached::from(OutAttachment {
        filename: "report.pdf".into(),
        data: vec![1],
    }));
    assert_eq!(send_guard(&d), None);
    // A forward with nothing attached is the user's choice to make.
    d.attachments.clear();
    d.edit(Field::Body, "FYI");
    assert_eq!(send_guard(&d), None);
    // No recipient: To says so until one is typed.
    d.edit(Field::To, "");
    assert_eq!(
        d.check_send(false),
        SendCheck::Blocked(SendBlock::NoRecipient)
    );
    assert!(d.need_recipient);
    d.edit(Field::To, "a");
    assert!(!d.need_recipient);
}

#[test]
fn an_untouched_draft_is_replaced_and_a_typed_one_kept() {
    let mut drafts = Drafts::default();
    let first = drafts.open(Draft::new(1));
    let second = drafts.open(Draft::new(1));
    assert_eq!(drafts.len(), 1, "an empty draft is not kept");
    assert!(drafts.get(first).is_none());
    drafts.active_mut().unwrap().edit(Field::Body, "hello");
    let third = drafts.open(Draft::new(1));
    assert_eq!(drafts.len(), 2);
    assert_eq!(drafts.newest_kept(), Some(second));
    drafts.remove(third);
    assert_eq!(drafts.active_id(), None);
    assert_eq!(drafts.newest_kept(), Some(second));
    // Typing then clearing everything leaves nothing to keep.
    let d = drafts.get_mut(second).unwrap();
    d.edit(Field::Body, " ");
    assert!(!d.keep());
}

#[test]
fn quotes_fold_out_of_the_composer_and_are_still_sent() {
    let m = message("Bob <bob@x.org>", "al@x.org", "");
    let body = Body {
        text: "Noon?\n> earlier".into(),
        ..Default::default()
    };
    // A reply: what is typed stays, the attribution and quote fold.
    let mut r = Draft::reply(&m, &body, "al@x.org", false, &Heads::en());
    r.sign("\n\n-- \nAl");
    let full = r.full_body();
    r.fold_quote();
    assert_eq!(r.body, "\n\n-- \nAl");
    assert!(r.quote.as_deref().unwrap().starts_with("\n\nOn "));
    assert!(r.quote_folded());
    assert_eq!(r.full_body(), full);
    r.body.insert_str(0, "Ok");
    assert_eq!(r.to_row(0).body, format!("Ok{full}"));
    // What is sent (outgoing's text part) is the full body.
    let sent = mail::compose_html::alternative(&r.full_body(), r.quoted_html.as_deref()).0;
    assert!(
        sent.starts_with("Ok\n") && sent.ends_with(">> earlier"),
        "{sent}"
    );

    // The body replaced as a whole (Ctrl+A, then typing) keeps its
    // distance from the quote, and reopening folds the same way.
    r.body = "Thanks.".into();
    let typed = r.full_body();
    assert!(typed.starts_with("Thanks.\n\nOn "), "{typed}");
    let mut again = Draft::new(1);
    again.kind = Kind::Reply;
    again.body = typed.clone();
    again.fold_quote();
    assert_eq!(again.body, "Thanks.");
    assert_eq!(again.full_body(), typed);
    assert_eq!(above_quote(&typed), "Thanks.");
    // A quote folded without blank lines above it still gets one.
    assert_eq!(
        with_quote("Hi", "On Mon, Bob wrote:\n> a"),
        "Hi\n\nOn Mon, Bob wrote:\n> a"
    );
    assert_eq!(
        with_quote("", "On Mon, Bob wrote:\n> a"),
        "On Mon, Bob wrote:\n> a"
    );

    // A forward, with its heading in any language.
    let pt = Heads {
        forwarded: "---------- Mensagem reencaminhada ----------".into(),
        from: "De".into(),
        date: "Data".into(),
        subject: "Assunto".into(),
        to: "Para".into(),
        ..Heads::en()
    };
    let mut f = Draft::forward(&m, &body, Vec::new(), &pt);
    f.fold_quote();
    assert_eq!(f.body, "");
    let q = f.quote.clone().unwrap();
    assert!(
        q.starts_with("\n\n---------- Mensagem reencaminhada ----------\nDe: "),
        "{q}"
    );
    // The words "anexo" in a forwarded message are not the user's.
    assert!(!mentions_attachment(
        "Hi\n---------- Mensagem reencaminhada ----------\nvai em anexo"
    ));

    // Unfolding gives the same text back; nothing to fold: no change.
    assert!(f.unfold_quote());
    assert!(!f.unfold_quote());
    let mut n = Draft::new(1);
    n.body = "Hi\n> not a quote of ours".into();
    n.fold_quote();
    assert!(n.quote.is_none());
    // Text above a reply's quote that ends in its own quoted lines.
    assert_eq!(
        quote_start("Hi\n\nOn Mon, Bob wrote:\n> a\n>\n> b\n"),
        Some(2)
    );
    assert_eq!(quote_start("Hi\nOn Mon, Bob wrote:\n> a"), Some(2));
    assert_eq!(quote_start("> only quotes\n> here"), None);
    assert_eq!(quote_start("Hi\n\n-- \n> sig"), None);
}

#[test]
fn the_preview_draws_what_is_sent_within_its_caps() {
    let html = |body: &str| mail::compose_html::preview_html(body, None);
    let (short, tall, wide) = std::thread::Builder::new()
        .stack_size(PREVIEW_STACK)
        .spawn(move || {
            (
                preview_page(html("Hi **Ana**"), 600.0, 1.0, false),
                preview_page(html(&"A line.\n\n".repeat(2000)), 600.0, 1.0, false),
                preview_page(html("Wide"), 1600.0, 1.5, false),
            )
        })
        .unwrap()
        .join()
        .unwrap();
    let short = short.unwrap();
    assert!(short.height > 0.0 && !short.cut);
    assert_eq!((short.pixmap.width(), short.width), (600, 600.0));
    assert!(
        short.pixmap.data().iter().any(|b| *b != 0),
        "something drawn"
    );
    // Its words for screen readers, a line each, on the page.
    let words: String = short.lines.iter().map(|l| l.text.as_str()).collect();
    assert!(words.contains("Ana"), "{:?}", short.lines);
    assert!(short.lines.iter().all(|l| l.y < short.height && l.w > 0.0));
    // One band at most: the rest is said to be cut, and so are its lines.
    let tall = tall.unwrap();
    assert!(tall.cut);
    assert_eq!(tall.pixmap.height(), PREVIEW_MAX_ROWS);
    assert!(tall.lines.iter().all(|l| l.y < tall.height));
    // Never laid out wider than 960 logical px (then drawn unstretched).
    let wide = wide.unwrap();
    assert_eq!(wide.width, 960.0);
    assert_eq!(wide.pixmap.width(), 1440);
}

#[test]
fn the_undo_send_offer_shortens_the_subject() {
    let none = "(no subject)";
    assert_eq!(short_subject("  Plans ", none), "Plans");
    assert_eq!(short_subject("", none), "(no subject)");
    let long = "A subject that goes on and on well past the forty characters";
    let cut = short_subject(long, none);
    assert_eq!(cut, "A subject that goes on and on well past\u{2026}");
    assert_eq!(cut.chars().count(), OFFER_SUBJECT_CHARS);
    assert_eq!(
        short_subject(&"ç".repeat(50), none).chars().count(),
        OFFER_SUBJECT_CHARS + 1
    );
}

#[test]
fn subject_prefixes() {
    assert_eq!(prefixed("Re:", "Hello"), "Re: Hello");
    assert_eq!(prefixed("Re:", "RE: Hello"), "RE: Hello");
    assert_eq!(prefixed("Fwd:", "  "), "Fwd: ");
}

#[test]
fn quoting() {
    assert_eq!(quote("a\n> b"), "> a\n>> b");
}

#[test]
fn signature_blocks() {
    assert_eq!(signature_block(""), "");
    assert_eq!(signature_block(" \r\n "), "");
    assert_eq!(signature_block("Ana\r\nACME  \n\n"), "\n\n-- \nAna\nACME");
    // A typed delimiter is not doubled.
    assert_eq!(signature_block("-- \nAna"), "\n\n-- \nAna");
    assert_eq!(signature_block("--\nAna"), "\n\n-- \nAna");
}

#[test]
fn signature_goes_in_once_per_kind() {
    let block = signature_block("Ana");
    let mut new = Draft::new(1);
    new.sign(&block);
    assert_eq!(new.body, "\n\n-- \nAna");
    // Only the signature: nothing written yet.
    assert!(new.is_empty() && !new.keep());

    let m = message("Bob <bob@x.org>", "ana@x.org", "");
    let body = Body {
        text: "See you at noon.\n-- \nBob\nBob's Bakery".into(),
        ..Default::default()
    };
    let mut reply = Draft::reply(&m, &body, "ana@x.org", false, &Heads::en());
    reply.sign(&block);
    // Above the quote, and the original's signature is not quoted.
    assert!(
        reply.body.starts_with("\n\n-- \nAna\n\nOn "),
        "{}",
        reply.body
    );
    assert!(
        reply.body.ends_with("wrote:\n> See you at noon."),
        "{}",
        reply.body
    );
    assert_eq!(reply.body.matches("-- ").count(), 1);

    let mut fwd = Draft::forward(&m, &body, Vec::new(), &Heads::en());
    fwd.sign(&block);
    assert!(
        fwd.body
            .starts_with(&format!("\n\n-- \nAna\n\n{FORWARDED}"))
    );
    // Forwarded as it was, signature included.
    assert!(fwd.body.ends_with("Bob's Bakery"));

    // A mailto: body comes first, then the signature.
    let mut mailto = Draft {
        body: "Hello".into(),
        ..Draft::new(1)
    };
    mailto.sign(&block);
    assert_eq!(mailto.body, "Hello\n\n-- \nAna");
    assert!(!mailto.is_empty());

    // No signature: nothing added.
    let mut plain = Draft::new(1);
    plain.sign("");
    assert_eq!(plain.body, "");
}

#[test]
fn signature_swaps_only_while_untouched() {
    let (ana, work) = (signature_block("Ana"), signature_block("Ana\nACME"));
    let mut d = Draft::new(1);
    d.sign(&ana);
    d.edit(Field::Body, &format!("Hi Bob{ana}"));
    assert!(d.resign(&work));
    assert_eq!(d.body, "Hi Bob\n\n-- \nAna\nACME");
    // Back, and to an account without one.
    assert!(d.resign(&ana));
    assert!(d.resign(""));
    assert_eq!(d.body, "Hi Bob");
    assert!(d.resign(&work));
    assert_eq!(d.body, "Hi Bob\n\n-- \nAna\nACME");

    // Edited signature: left alone from then on.
    d.edit(Field::Body, "Hi Bob\n\n-- \nAna (mobile)\nACME");
    assert!(!d.resign(&ana));
    assert_eq!(d.body, "Hi Bob\n\n-- \nAna (mobile)\nACME");

    // A reply without a signature gets one above the quote, unless
    // something was typed there.
    let m = message("bob@x.org", "ana@x.org", "");
    let mut r = Draft::reply(&m, &Body::default(), "ana@x.org", false, &Heads::en());
    r.sign("");
    assert!(r.resign(&ana));
    assert!(r.body.starts_with("\n\n-- \nAna\n\nOn "), "{}", r.body);
    let mut r = Draft::reply(&m, &Body::default(), "ana@x.org", false, &Heads::en());
    r.sign("");
    r.body.insert_str(0, "Thanks!");
    assert!(!r.resign(&ana));

    // A saved draft whose signature isn't known is never touched.
    let mut saved = Draft::new(1);
    saved.body = "Hi\n\n-- \nOld".into();
    assert!(!saved.resign(&ana));
}

#[test]
fn html_quotes_follow_the_text_quote() {
    use mail::compose_html::alternative;
    let mut m = message("bob@x.org", "al@x.org", "");
    m.from_name = "Bob".into();
    m.subject = "Lunch".into();
    let body = Body {
        text: "Noon?".into(),
        ..Default::default()
    };
    // As the app does it: the quote, then the signature above it.
    let mut reply = Draft::reply(&m, &body, "al@x.org", false, &Heads::en());
    reply.quote_html(&m, "<p>Noon?</p>", &Heads::en());
    reply.sign(&signature_block("Ana"));
    let typed = format!("Yes{}", reply.body);
    reply.edit(Field::Body, &typed);
    let (text, html) = alternative(&reply.body, reply.quoted_html.as_deref());
    assert_eq!(text, typed);
    let html = html.unwrap();
    assert!(
        html.contains(
            "<p>Yes</p><div class=\"sig\">-- <br>Ana</div><div class=\"komorebi_quote\" style=\"margin-top:1em\">On "
        ),
        "{html}"
    );
    assert!(html.contains("<p>Noon?</p></blockquote>"), "{html}");

    let mut fwd = Draft::forward(&m, &body, Vec::new(), &Heads::en());
    fwd.quote_html(&m, "<p>Noon?</p>", &Heads::en());
    let html = alternative(&fwd.body, fwd.quoted_html.as_deref())
        .1
        .unwrap();
    assert!(
        html.contains("<b>From:</b></td><td>Bob &lt;bob@x.org&gt;</td>"),
        "{html}"
    );
    assert!(
        html.contains("<b>Subject:</b></td><td>Lunch</td>"),
        "{html}"
    );
    assert!(html.contains("</table><br><p>Noon?</p></div>"), "{html}");
    // The heading deleted: the text goes as typed.
    let cut = fwd.body.replace(FORWARDED, "");
    let html = alternative(&cut, fwd.quoted_html.as_deref()).1.unwrap();
    assert!(!html.contains("komorebi_quote"), "{html}");

    let mut new = Draft::new(1);
    new.quote_html(&m, "<p>Noon?</p>", &Heads::en());
    assert_eq!(new.quoted_html, None);
}

#[test]
fn only_a_short_trailing_signature_is_stripped() {
    assert_eq!(strip_signature("Hi\n\n-- \nBob\n"), "Hi");
    assert_eq!(strip_signature("Hi\r\n-- \r\nBob\r\n"), "Hi");
    // The last delimiter counts.
    assert_eq!(strip_signature("A\n-- \nx\nB\n-- \nBob"), "A\n-- \nx\nB");
    // "--" without the space, or "-- " inside a line, is text.
    assert_eq!(strip_signature("Hi\n--\nBob"), "Hi\n--\nBob");
    assert_eq!(strip_signature("a -- b"), "a -- b");
    // A long tail (a quoted thread under a signature) stays.
    let long = format!("Hi\n-- \nBob\n{}", "> old\n".repeat(20));
    assert_eq!(strip_signature(&long), long);
}

/// Komorebi sets no attachment limit (owner decision, 2026-11-16): a file
/// is read whole, however much is attached already, and no size stops
/// Send. The server's "too large" refusal is the only limit.
#[test]
fn attachments_have_no_size_limit() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let len = std::fs::metadata(&path).unwrap().len();
    assert_eq!(read_attachment(&path).unwrap().len() as u64, len);
    let mut d = Draft {
        to: "ana@example.org".into(),
        ..Default::default()
    };
    let file = |len: usize| {
        Attached::from(OutAttachment {
            filename: "a.bin".into(),
            data: vec![0; len],
        })
    };
    // Past the old 25 MB send block and the old 100 MB (encoded) cap.
    let big = 76 * 1024 * 1024;
    assert!(d.add_attachments(vec![file(big)]));
    assert!(d.add_attachments(vec![file(1)]));
    assert_eq!(d.attachments.len(), 2);
    assert_eq!(attachments_size(&d.attachments), big as u64 + 1);
    assert_eq!(d.send_block(), None);
    assert_eq!(d.check_send(true), SendCheck::Ready);
    assert!(d.touched && d.dirty);
    assert!(!d.add_attachments(Vec::new()), "nothing added");
}

#[test]
fn dropped_paths_attach_files_only() {
    let dir = std::env::temp_dir().join(format!("komorebi-drop-lib-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("folder")).unwrap();
    std::fs::write(dir.join("Relatório.txt"), "olá").unwrap();
    let paths = [
        dir.join("Relatório.txt"),
        dir.join("folder"),
        dir.join("missing.pdf"),
    ];
    let (added, errors) = read_files(&paths);
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].file.filename, "Relatório.txt");
    assert_eq!(added[0].file.data, "olá".as_bytes());
    assert_eq!(errors.len(), 2);
    assert_eq!(
        errors[0],
        AttachFail {
            name: "folder".into(),
            why: AttachWhy::Folder
        }
    );
    assert_eq!(errors[1].name, "missing.pdf");
    assert!(matches!(errors[1].why, AttachWhy::Io(_)), "{:?}", errors[1]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_caret_follows_the_edit() {
    assert_eq!(caret_after_edit("", "a"), 1);
    assert_eq!(caret_after_edit("ana, b", "ana, bo"), 7);
    // Typed in the middle of a list.
    assert_eq!(caret_after_edit("a@x, b@x", "ajo@x, b@x"), 3);
    assert_eq!(caret_after_edit("joão, b", "joãoz, b"), 6);
    assert_eq!(caret_after_edit("abc", "ac"), 1);
    assert_eq!(caret_after_edit("ana", "x"), 1);
    assert_eq!(caret_after_edit("ana", ""), 0);
}

#[test]
fn suggestions_complete_the_address_at_the_caret() {
    // The last address of a list.
    let text = "ana@x.pt, jo";
    assert_eq!(typed_address(text, text.len()), "jo");
    let (out, caret) = complete_address(text, text.len(), "João <joao@x.pt>");
    assert_eq!(out, "ana@x.pt, João <joao@x.pt>, ");
    assert_eq!(caret, out.len());
    // The caret in the middle of a list: the separator after the
    // address is reused and the rest stays.
    let text = "ana@x.pt; jo, bob@x.pt";
    let cursor = text.find(',').unwrap();
    assert_eq!(typed_address(text, cursor), "jo");
    let (out, caret) = complete_address(text, cursor, "João <joao@x.pt>");
    assert_eq!(out, "ana@x.pt; João <joao@x.pt>, bob@x.pt");
    assert_eq!(&out[caret..], "bob@x.pt");
    // Only what is typed before the caret is matched.
    assert_eq!(typed_address("johnny", 2), "jo");
    assert_eq!(complete_address("johnny", 2, "J <j@x>").0, "J <j@x>, ");
    // A quoted name with a comma is one address.
    let text = r#""Doe, Jane" <jane@x.pt>, "Sil"#;
    assert_eq!(typed_address(text, text.len()), "Sil");
    let quoted = smtp::mailbox("Silva, Ana", "ana@x.pt");
    let (out, _) = complete_address(text, text.len(), &quoted);
    assert_eq!(out, r#""Doe, Jane" <jane@x.pt>, "Silva, Ana" <ana@x.pt>, "#);
    assert_eq!(smtp::split_addresses(&out).len(), 2);
    assert_eq!(typed_address(r#""Doe, Ja"#, 8), "Doe, Ja");
    // Any caret, even one off the text or inside a character.
    for cursor in 0..12 {
        let _ = complete_address("ã, é", cursor, "x@y");
        let _ = typed_address("ã, é", cursor);
    }
    assert_eq!(complete_address("", 0, "x@y"), ("x@y, ".to_string(), 5));
}

#[test]
fn reply_recipients_cases() {
    let m = message(
        "bob@x.org",
        "Al <AL@x.org>, val@x.org, bob@x.org",
        "al@x.org, cy@x.org",
    );
    let me = "al@x.org";
    assert_eq!(
        reply_recipients(&m, "", me, false),
        (vec!["bob@x.org".into()], vec![])
    );
    // val@ is not mistaken for al@, and the sender is not repeated.
    assert_eq!(
        reply_recipients(&m, "", me, true),
        (
            vec!["bob@x.org".into(), "val@x.org".into()],
            vec!["cy@x.org".into()]
        )
    );
    assert_eq!(
        reply_recipients(&m, "list@x.org", me, false).0,
        ["list@x.org"]
    );
    // Replying to a Sent item goes to its original recipients.
    let sent = message("Al@x.org", "Bob <bob@x.org>, val@x.org", "cy@x.org");
    assert_eq!(
        reply_recipients(&sent, "", me, false),
        (vec!["Bob <bob@x.org>".into(), "val@x.org".into()], vec![])
    );
    assert_eq!(
        reply_recipients(&sent, "", me, true),
        (
            vec!["Bob <bob@x.org>".into(), "val@x.org".into()],
            vec!["cy@x.org".into()]
        )
    );
    assert_eq!(bare_address("\"a <b>\" <C@x.org> "), "c@x.org");
}

// ---------------------------------------------------- saved in the cache

/// A throwaway cache with one account, removed with its drafts folder.
pub(crate) struct Cache {
    pub db: Db,
    pub account: i64,
    /// Declared after `db`, so the cache is closed before the files go.
    pub path: TempPath,
}

pub(crate) struct TempPath(PathBuf);

impl std::ops::Deref for TempPath {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        remove_db(&self.0);
    }
}

fn remove_db(path: &Path) {
    for ext in ["", "-wal", "-shm"] {
        let mut p = path.as_os_str().to_owned();
        p.push(ext);
        std::fs::remove_file(p).ok();
    }
    std::fs::remove_dir_all(db::drafts_dir(path)).ok();
}

impl Cache {
    pub fn new(name: &str) -> Self {
        let path = crate::testdir::unique("komorebi-compose", name).with_extension("sqlite3");
        remove_db(&path);
        let db = Db::open(&path).unwrap();
        let server = |host: &str| Server {
            host: host.into(),
            port: 993,
            security: Security::Tls,
        };
        let account = db
            .add_account(&NewAccount {
                imap_user: String::new(),
                smtp_user: String::new(),
                email: "me@example.org".into(),
                name: "Me".into(),
                imap: server("imap.example.invalid"),
                smtp: server("smtp.example.invalid"),
                save_sent: true,
                smtp_no_auth: false,
            })
            .unwrap();
        Cache {
            db,
            account,
            path: TempPath(path),
        }
    }
}

fn typed(account: i64) -> Draft {
    let mut d = Draft::new(account);
    d.edit(Field::To, "Ana <ana@example.org>");
    d.edit(Field::Subject, "Plans");
    d.edit(Field::Body, "Line one\nLine two");
    d
}

#[test]
fn empty_and_untouched_drafts_are_not_saved() {
    let c = Cache::new("empty");
    let mut d = Draft::new(c.account);
    assert_eq!(
        store(&c.db, &mut d, "me@example.org").unwrap(),
        Stored::Nothing
    );
    // Typed, then cleared again: still nothing.
    d.edit(Field::Body, "x");
    d.edit(Field::Body, " ");
    assert_eq!(
        store(&c.db, &mut d, "me@example.org").unwrap(),
        Stored::Nothing
    );
    // A reply only the app filled in is not saved either.
    let mut m = message("bob@x.org", "me@example.org", "");
    m.account_id = c.account;
    let mut reply = Draft::reply(&m, &Body::default(), "me@example.org", false, &Heads::en());
    assert!(!reply.is_empty());
    assert_eq!(
        store(&c.db, &mut reply, "me@example.org").unwrap(),
        Stored::Nothing
    );
    assert_eq!(c.db.count_drafts().unwrap(), 0);
    assert!(d.row.is_none() && d.message_id.is_empty());
}

#[test]
fn importance_is_saved_and_sent() {
    use db::Importance;
    let c = Cache::new("importance");
    let mut d = typed(c.account);
    assert!(d.set_importance(Importance::High));
    assert!(!d.set_importance(Importance::High));
    assert_eq!(
        store(&c.db, &mut d, "me@example.org").unwrap(),
        Stored::Inserted
    );
    let row = d.row.expect("saved");
    let (back, _) = load(&Db::open(&c.path).unwrap(), row).unwrap().unwrap();
    assert_eq!(back.importance, Importance::High);
    // Sent with it.
    let account = c.db.accounts().unwrap().remove(0);
    let raw = smtp::build_raw(outgoing(&c.db, &account, back)).unwrap();
    assert_eq!(mail::mime::parse_header(&raw).importance, Importance::High);
    // Back to normal: saved, and sent with neither header.
    assert!(d.set_importance(Importance::Normal));
    assert_eq!(
        store(&c.db, &mut d, "me@example.org").unwrap(),
        Stored::Updated
    );
    let (back, _) = load(&c.db, row).unwrap().unwrap();
    assert_eq!(back.importance, Importance::Normal);
    let raw = smtp::build_raw(outgoing(&c.db, &account, back)).unwrap();
    let raw = String::from_utf8(raw).unwrap();
    assert!(!raw.contains("Importance:") && !raw.contains("X-Priority:"));
    // Importance alone does not make a blank message worth keeping.
    let mut blank = Draft::new(c.account);
    assert!(blank.set_importance(Importance::Low));
    assert!(!blank.touched && !blank.keep());
}

#[test]
fn saved_drafts_round_trip_with_a_stable_message_id() {
    let c = Cache::new("round-trip");
    let mut d = typed(c.account);
    d.kind = Kind::ReplyAll;
    d.edit(Field::Cc, "cy@example.org");
    d.edit(Field::Bcc, "dee@example.org");
    d.reply_to = None;
    d.in_reply_to = "m1@x.org".into();
    d.references = "<m0@x.org> <m1@x.org>".into();
    d.quoted_html = Some("<!--komorebi-quote 1 0-->quote".into());
    d.attachments.push(
        OutAttachment {
            filename: "notes.txt".into(),
            data: b"hello".to_vec(),
        }
        .into(),
    );
    assert_eq!(
        store(&c.db, &mut d, "me@example.org").unwrap(),
        Stored::Inserted
    );
    let row = d.row.expect("saved");
    let id = d.message_id.clone();
    assert!(id.ends_with("@example.org>"), "{id}");
    assert!(!d.dirty);
    let file = d.attachments[0].saved.expect("copied");
    // Nothing changed: nothing written. An edit is one update; the
    // attachment is not copied again.
    assert_eq!(
        store(&c.db, &mut d, "me@example.org").unwrap(),
        Stored::Nothing
    );
    d.edit(Field::Body, "Line one\nLine two\nLine three");
    assert_eq!(
        store(&c.db, &mut d, "me@example.org").unwrap(),
        Stored::Updated
    );
    assert_eq!(c.db.draft_files(row).unwrap().len(), 1);
    assert_eq!(d.attachments[0].saved, Some(file));

    // As after a restart: every field, the attachment and the same
    // Message-ID come back.
    let (back, missing) = load(&Db::open(&c.path).unwrap(), row).unwrap().unwrap();
    assert!(missing.is_empty());
    assert_eq!(back.row, Some(row));
    assert_eq!(back.kind, Kind::ReplyAll);
    assert_eq!(back.to, "Ana <ana@example.org>");
    assert_eq!(back.cc, "cy@example.org");
    assert_eq!(back.bcc, "dee@example.org");
    assert!(back.show_cc);
    assert_eq!(back.subject, "Plans");
    assert_eq!(back.body, "Line one\nLine two\nLine three");
    assert_eq!(back.in_reply_to, "m1@x.org");
    assert_eq!(back.references, "<m0@x.org> <m1@x.org>");
    assert_eq!(back.quoted_html, d.quoted_html);
    assert_eq!(back.message_id, id);
    assert_eq!(back.attachments.len(), 1);
    assert_eq!(back.attachments[0].file.filename, "notes.txt");
    assert_eq!(back.attachments[0].file.data, b"hello");
    assert_eq!(back.attachments[0].saved, Some(file));
    assert!(back.keep() && !back.dirty);
    let mut back = back;
    assert_eq!(
        store(&c.db, &mut back, "other@example.org").unwrap(),
        Stored::Nothing
    );
    assert_eq!(back.message_id, id);

    // A copy that vanished is left out and named.
    std::fs::remove_file(
        db::drafts_dir(&c.path)
            .join(row.to_string())
            .join(file.to_string()),
    )
    .unwrap();
    let (back, missing) = load(&c.db, row).unwrap().unwrap();
    assert!(back.attachments.is_empty());
    assert_eq!(missing, ["notes.txt"]);
}

#[test]
fn emptying_a_saved_draft_deletes_it_and_its_copies() {
    let c = Cache::new("emptied");
    let mut d = typed(c.account);
    d.attachments.push(
        OutAttachment {
            filename: "a.bin".into(),
            data: vec![7; 10],
        }
        .into(),
    );
    store(&c.db, &mut d, "me@example.org").unwrap();
    let row = d.row.unwrap();
    let folder = db::drafts_dir(&c.path).join(row.to_string());
    assert!(folder.exists());
    for field in [Field::To, Field::Subject, Field::Body] {
        d.edit(field, "");
    }
    // Removing the attachment removes its copy at once.
    assert!(d.remove_attachment(&c.db, 0).unwrap());
    assert!(!d.remove_attachment(&c.db, 0).unwrap());
    assert!(c.db.draft_files(row).unwrap().is_empty());
    assert_eq!(
        store(&c.db, &mut d, "me@example.org").unwrap(),
        Stored::Deleted(None)
    );
    assert!(d.row.is_none());
    assert!(c.db.draft(row).unwrap().is_none());
    assert!(!folder.exists());
    assert!(load(&c.db, row).unwrap().is_none());
}

#[test]
fn edits_within_two_seconds_end_in_one_save() {
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    // The timer starts with the first edit and comes due 2 s later.
    let edits = [0, 400, 1_500];
    let mut last = at(0);
    let mut due = at(2_000);
    let mut saves = Vec::new();
    for _ in 0..10 {
        for &e in &edits {
            if at(e) <= due {
                last = last.max(at(e));
            }
        }
        match autosave_wait(last, due) {
            Some(rest) => due += rest,
            None => {
                saves.push(due.duration_since(t0).as_millis());
                break;
            }
        }
    }
    assert_eq!(saves, [3_500]);
    assert_eq!(autosave_wait(at(0), at(2_000)), None);
    assert_eq!(autosave_wait(at(0), at(5_000)), None);
    assert_eq!(
        autosave_wait(at(1_000), at(2_000)),
        Some(Duration::from_millis(1_000))
    );
}

#[test]
fn server_copies_go_at_most_every_thirty_seconds() {
    let t0 = Instant::now();
    let at = |s: u64| t0 + Duration::from_secs(s);
    // An autosave every 2 s for 70 s of typing.
    let mut last = None;
    let mut copies = Vec::new();
    for s in (2..=70).step_by(2) {
        if server_save_due(last, at(s)) {
            last = Some(at(s));
            copies.push(s);
        }
    }
    assert_eq!(copies, [2, 32, 62]);
    assert!(server_save_due(None, t0));
    assert!(!server_save_due(Some(at(10)), at(39)));
    assert!(server_save_due(Some(at(10)), at(40)));
}

// ------------------------------------------------- the formatted editor

#[test]
fn a_switch_to_plain_text_comes_back_unchanged() {
    let defaults = ComposeDefaults::default().doc();
    let mut d = Draft::new(1);
    d.make_rich(defaults, None);
    let ed = d.rich.as_mut().unwrap();
    ed.replace(0, 0, "Hello", (0, 0)).unwrap();
    let len = ed.text().len();
    assert!(rich::apply(ed, (0, len), &rich::Action::Bold).changed);
    assert!(d.set_rich(false, defaults));
    assert_eq!(d.body, "Hello");
    assert!(d.can_unplain());
    // Back before typing: the bold is still there.
    assert!(d.set_rich(true, defaults));
    let doc = d.rich.as_ref().unwrap().doc();
    assert!(model::needs_html(doc));
    // Typed in plain text since: a new document from the text.
    assert!(d.set_rich(false, defaults));
    d.edit(Field::Body, "Hello there");
    assert!(!d.can_unplain());
    assert!(d.set_rich(true, defaults));
    assert!(!model::needs_html(d.rich.as_ref().unwrap().doc()));
    assert_eq!(d.full_body().trim_end(), "Hello there");
}

#[test]
fn toolbar_actions_change_the_document_and_undo() {
    let mut ed = Editor::new(rich::doc_from_text(
        "One\nTwo",
        ComposeDefaults::default().doc(),
    ));
    let all = (0, ed.text().len());
    assert!(rich::apply(&mut ed, all, &rich::Action::Bullets).changed);
    assert!(rich::apply(&mut ed, (0, 3), &rich::Action::Italic).changed);
    let f = ed.format_at(0, 3);
    assert!(f.i && f.list.is_some());
    let undone = rich::apply(&mut ed, (0, 0), &rich::Action::Undo);
    assert!(undone.changed && undone.sel.is_some());
    assert!(!ed.format_at(0, 3).i);
    // Nothing to redo past the end; a link needs a followable address.
    assert!(rich::apply(&mut ed, (0, 0), &rich::Action::Redo).changed);
    assert!(!rich::apply(&mut ed, (0, 0), &rich::Action::Redo).changed);
    assert_eq!(rich::link(&mut ed, (0, 3), "not a link"), None);
    assert!(rich::link(&mut ed, (0, 3), "example.org").is_some());
    assert!(ed.format_at(0, 3).link.is_some());
}

/// The format bar's boxes: a selection with one font, size, colour and
/// alignment shows them; one with two of any says it is mixed (the box
/// then shows no single value), and a caret is never mixed.
#[test]
fn the_format_says_when_a_selection_is_mixed() {
    use crate::mail::rich::{Align, ColorId, FontId, SizePt};
    let mut ed = Editor::new(rich::doc_from_text(
        "One\nTwo",
        ComposeDefaults::default().doc(),
    ));
    let all = (0, ed.text().len());
    let f = ed.format_at(all.0, all.1);
    assert_eq!(f.mixed, Default::default(), "one of each");
    let big = SizePt::new(18).unwrap();
    assert!(rich::apply(&mut ed, (0, 3), &rich::Action::Font(FontId::Georgia)).changed);
    assert!(rich::apply(&mut ed, (0, 3), &rich::Action::Size(big)).changed);
    assert!(rich::apply(&mut ed, (0, 3), &rich::Action::Color(Some(ColorId::Blue))).changed);
    assert!(rich::apply(&mut ed, (0, 3), &rich::Action::Align(Align::Center)).changed);
    // The first paragraph alone: one of each, the new ones.
    let f = ed.format_at(0, 3);
    assert_eq!(f.mixed, Default::default());
    assert_eq!(
        (f.font, f.size, f.align),
        (FontId::Georgia, big, Align::Center)
    );
    assert_eq!(f.color, Some(ColorId::Blue));
    // Both paragraphs: everything is mixed.
    let all = (0, ed.text().len());
    let f = ed.format_at(all.0, all.1);
    assert!(f.mixed.font && f.mixed.size && f.mixed.color && f.mixed.align);
    // A caret is one place: never mixed.
    let f = ed.format_at(1, 1);
    assert_eq!(f.mixed, Default::default());
    assert_eq!(f.font, FontId::Georgia);
}

#[test]
fn another_account_swaps_an_untouched_signature() {
    let c = Cache::new("switch");
    let other =
        c.db.add_account(&NewAccount {
            imap_user: String::new(),
            smtp_user: String::new(),
            email: "work@example.org".into(),
            name: "Work".into(),
            imap: Server {
                host: "imap.example.invalid".into(),
                port: 993,
                security: Security::Tls,
            },
            smtp: Server {
                host: "smtp.example.invalid".into(),
                port: 465,
                security: Security::Tls,
            },
            save_sent: true,
            smtp_no_auth: false,
        })
        .unwrap();
    let doc = rich::doc_from_text("Ana\nACME", ComposeDefaults::default().doc());
    let sig =
        c.db.add_signature("Work", &doc, &Default::default())
            .unwrap();
    c.db.set_account_signatures(other, Some(sig), Some(sig))
        .unwrap();
    let plain = ComposeDefaults {
        rich: false,
        ..ComposeDefaults::default()
    };
    let mut d = Draft::new(c.account);
    prepare(&c.db, &mut d, &plain);
    assert_eq!(d.body, "");
    let work = signature_for(&c.db, other, Kind::New);
    assert_eq!(d.switch_account(other, work.as_ref()), Some(true));
    assert_eq!(d.body, "\n\n-- \nAna\nACME");
    // Only the signature: still nothing written.
    assert!(!d.touched);
    assert_eq!(d.switch_account(other, work.as_ref()), None);
    // Picked from the Signature menu: none, then back.
    assert!(d.pick_signature(None));
    assert_eq!(d.body, "");
    assert!(d.pick_signature(work.as_ref()));
    assert_eq!(d.body, "\n\n-- \nAna\nACME");

    // Formatted: the signature block in the document follows.
    let mut f = Draft::new(other);
    prepare(&c.db, &mut f, &ComposeDefaults::default());
    assert_eq!(f.rich_signature().and_then(|s| s.source), Some(sig));
    assert_eq!(f.switch_account(c.account, None), Some(true));
    assert!(f.rich_signature().is_none());
}

#[test]
fn painted_pieces_go_to_straight_bgra() {
    let mut pm = tiny_skia::Pixmap::new(2, 1).unwrap();
    // Premultiplied RGBA: opaque red, then blue at half alpha.
    pm.data_mut()
        .copy_from_slice(&[255, 0, 0, 255, 0, 0, 128, 128]);
    let (w, h, data) = rich::straight_bgra(pm);
    assert_eq!((w, h), (2, 1));
    assert_eq!(&data[..4], &[0, 0, 255, 255], "BGRA order");
    assert_eq!(&data[4..], &[255, 0, 0, 128], "un-premultiplied");
}

/// A formatted draft with a picture and a large attachment: the rows are
/// recorded at once and the copies written on another thread (as the app
/// does, so a large file never holds the window); reading it back is done
/// in steps, the slow one on another thread too. Pictures it no longer
/// shows are not kept.
#[test]
fn pictures_and_attachments_are_saved_and_read_off_the_ui_thread() {
    use crate::mail::rich::images::tests::png;
    let c = Cache::new("pictures");
    let mut d = typed(c.account);
    d.make_rich(ComposeDefaults::default().doc(), None);
    let (file, w, h) = model::ImageFile::new(png(800, 600)).unwrap();
    let img = model::InlineImage::new(file.cid(), w, h, "Step 1");
    let (gone, gw, gh) = model::ImageFile::new(png(10, 10)).unwrap();
    {
        let ed = d.rich.as_mut().unwrap();
        let end = ed.text().len();
        let after = ed
            .insert_image(end, end, img.clone(), file.clone())
            .unwrap();
        // One added and taken out again: in memory for undo, never saved.
        let i2 = model::InlineImage::new(gone.cid(), gw, gh, "");
        let after2 = ed.insert_image(after, after, i2, gone.clone()).unwrap();
        ed.replace(after, after2, "", (after, after2)).unwrap();
        assert_eq!(ed.images().len(), 2);
    }
    let big = vec![3u8; 3 * 1024 * 1024];
    d.attachments.push(
        OutAttachment {
            filename: "big.bin".into(),
            data: big.clone(),
        }
        .into(),
    );
    let (stored, jobs) = store_rows(&c.db, &mut d, "me@example.org").unwrap();
    assert_eq!(stored, Stored::Inserted);
    assert_eq!(jobs.len(), 2, "the attachment and the one picture shown");
    let row = d.row.unwrap();
    let files = c.db.draft_files(row).unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(
        files
            .iter()
            .filter_map(|f| f.cid.as_deref())
            .collect::<Vec<_>>(),
        [img.cid.as_str()]
    );
    assert!(
        jobs.iter().all(|j| !j.copy.path.exists()),
        "not written yet"
    );
    std::thread::spawn(move || {
        for j in &jobs {
            j.run().unwrap();
        }
    })
    .join()
    .unwrap();

    let job = load_job(&c.db, row).unwrap().unwrap();
    let read = std::thread::spawn(move || job.run()).join().unwrap();
    let (mut back, missing) = finish_load(&c.db, read);
    assert!(missing.is_empty());
    // The picture is not an attachment; the attachment is whole.
    assert_eq!(back.attachments.len(), 1);
    assert_eq!(back.attachments[0].file.data, big);
    let ed = back.rich.as_ref().unwrap();
    assert_eq!(ed.doc(), d.rich.as_ref().unwrap().doc());
    assert_eq!(ed.shown_images(), vec![(img.cid.clone(), file.clone())]);
    assert_eq!(back.saved_images.len(), 1);
    // Unchanged: nothing written, nothing copied.
    let (stored, jobs) = store_rows(&c.db, &mut back, "me@example.org").unwrap();
    assert_eq!((stored, jobs.len()), (Stored::Nothing, 0));
    // The picture deleted: its saved copy goes with the next save.
    {
        let ed = back.rich.as_mut().unwrap();
        let at = ed.image_offsets()[0].0;
        ed.replace(at, at + 3, "", (at, at + 3)).unwrap();
    }
    back.dirty = true;
    assert_eq!(
        store(&c.db, &mut back, "me@example.org").unwrap(),
        Stored::Updated
    );
    let files = c.db.draft_files(row).unwrap();
    assert!(files.iter().all(|f| f.cid.is_none()), "{files:?}");
    assert!(back.saved_images.is_empty());
}

/// A copy that cannot be written loses its row, and the next save tries
/// again.
#[test]
fn a_copy_that_fails_is_tried_again() {
    let c = Cache::new("copy-fails");
    let mut d = typed(c.account);
    d.attachments.push(
        OutAttachment {
            filename: "a.txt".into(),
            data: b"a".to_vec(),
        }
        .into(),
    );
    let (_, jobs) = store_rows(&c.db, &mut d, "me@example.org").unwrap();
    assert_eq!(jobs.len(), 1);
    copy_failed(&c.db, &mut d, &jobs[0]);
    assert!(d.attachments[0].saved.is_none());
    assert!(c.db.draft_files(d.row.unwrap()).unwrap().is_empty());
    let (_, again) = store_rows(&c.db, &mut d, "me@example.org").unwrap();
    assert_eq!(again.len(), 1);
}
