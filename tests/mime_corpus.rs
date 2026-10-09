//! What the MIME layer hands the HTML renderer for the synthetic corpus:
//! which HTML alternative, which inline parts, and that the attachment list
//! and the text view are unchanged by it. Also which parts the attachment
//! list hides (inline images, a duplicate invite, signatures), including the
//! signed fixtures in `tests/fixtures/mime/`.

mod common;

use common::raw;
use komorebi::mail::mime;

fn cids(name: &str) -> Vec<(String, String)> {
    let mut v: Vec<_> = mime::parse_render_input(&raw(name))
        .inline
        .into_iter()
        .map(|p| (p.cid, p.mime))
        .collect();
    v.sort();
    v
}

fn attachment_names(name: &str) -> Vec<String> {
    mime::parse_body(&raw(name))
        .attachments
        .into_iter()
        .map(|a| a.name)
        .collect()
}

#[test]
fn campaign_html_and_inline_images() {
    let name = "01-mailchimp-campaign";
    let r = mime::parse_render_input(&raw(name));
    let html = r.html.as_deref().unwrap();
    assert!(html.contains("cid:hero-autumn@lumen.example.com"));
    assert!(!r.prefers_plain);
    assert_eq!(r.text, mime::parse_body(&raw(name)).text);
    let png = |c: &str| (format!("{c}@lumen.example.com"), "image/png".to_string());
    let jpg = |c: &str| (format!("{c}@lumen.example.com"), "image/jpeg".to_string());
    assert_eq!(
        cids(name),
        [
            jpg("hero-autumn"),
            png("logo-lumen"),
            jpg("prod-book"),
            jpg("prod-lamp"),
            jpg("prod-mug"),
            jpg("prod-plant"),
        ]
    );
    let logo = r.inline.iter().find(|p| p.cid.starts_with("logo")).unwrap();
    assert!(logo.bytes.starts_with(b"\x89PNG"));
    // The page draws every inline image, so none is listed as a file.
    assert!(attachment_names(name).is_empty());
}

#[test]
fn receipt_html_inside_related_alternative() {
    let name = "04-order-receipt";
    let r = mime::parse_render_input(&raw(name));
    assert!(r.html.as_deref().unwrap().contains("Marta Silva"));
    assert!(!r.prefers_plain);
    assert_eq!(cids(name).len(), 5);
    assert!(cids(name).contains(&(
        "logo-parcelo@parcelo.example.com".into(),
        "image/png".into()
    )));
    assert!(attachment_names(name).is_empty());
}

#[test]
fn newsletter_inline_images_are_not_attachments() {
    // Six named inline images, each referenced from the HTML.
    let name = "11-newsletter-2col";
    assert_eq!(cids(name).len(), 6);
    assert!(attachment_names(name).is_empty());
}

#[test]
fn calendar_invite_prefers_html_over_calendar() {
    let name = "13-calendar-invite";
    let r = mime::parse_render_input(&raw(name));
    let html = r.html.as_deref().unwrap();
    assert!(html.contains("<html") || html.contains("<table"), "{html}");
    assert!(!html.contains("BEGIN:VCALENDAR"));
    assert!(!r.prefers_plain);
    assert!(r.inline.is_empty());
    // The text/calendar alternative is the same event as invite.ics.
    assert_eq!(attachment_names(name), ["invite.ics"]);
    let (file, data) = mime::attachment(&raw(name), 0).unwrap();
    assert_eq!(file, "invite.ics");
    assert!(String::from_utf8_lossy(&data).contains("UID:evt123@calendar.example.com"));
}

#[test]
fn adversarial_first_html_alternative_and_hidden_parts() {
    let name = "14-adversarial-malformed";
    let r = mime::parse_render_input(&raw(name));
    let html = r.html.as_deref().unwrap();
    assert!(html.contains("Unclosed attribute quote"));
    assert!(!html.contains("Second HTML alternative"));
    assert!(!r.prefers_plain);
    assert_eq!(
        cids(name),
        [
            ("inline-ok@adv.example.com".into(), "image/jpeg".into()),
            ("not-really-png@adv.example.com".into(), "image/png".into()),
            ("truncated@adv.example.com".into(), "image/jpeg".into()),
        ]
    );
    // plant.jpg is drawn (referenced), and the truncated JPEG has no file
    // name, so the list hides both; both still reach the renderer.
    // lies.png is not referenced, so it stays a file.
    assert_eq!(attachment_names(name), ["lies.png"]);
    assert_eq!(r.text, mime::parse_body(&raw(name)).text);
}

#[test]
fn text_matches_parse_body_across_corpus() {
    for name in common::FILES {
        let raw = raw(name);
        let r = mime::parse_render_input(&raw);
        assert!(r.html.is_some(), "{name}");
        assert_eq!(r.text, mime::parse_body(&raw).text, "{name}");
    }
}

#[test]
fn reader_parse_matches_the_separate_parses() {
    for name in common::FILES {
        let raw = raw(name);
        let (body, input) = mime::parse_reader(&raw);
        let alone = mime::parse_body(&raw);
        assert_eq!(body.text, alone.text, "{name}");
        assert_eq!(body.attachments, alone.attachments, "{name}");
        assert_eq!(body.references, alone.references, "{name}");
        assert_eq!(body.reply_to, alone.reply_to, "{name}");
        let r = mime::parse_render_input(&raw);
        assert_eq!(input.html, r.html, "{name}");
        assert_eq!(input.text, r.text, "{name}");
        assert_eq!(input.prefers_plain, r.prefers_plain, "{name}");
        assert_eq!(input.inline.len(), r.inline.len(), "{name}");
    }
}

fn signed(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mime")
        .join(name);
    std::fs::read(path).unwrap()
}

#[test]
fn smime_signed_mail_hides_the_signature_and_shows_the_body() {
    let raw = signed("smime-signed.eml");
    let body = mime::parse_body(&raw);
    assert!(body.attachments.is_empty(), "{:?}", body.attachments);
    assert!(body.text.starts_with("Hi Tiago,"), "{}", body.text);
    assert!(body.text.contains("quarterly figures"));
    let r = mime::parse_render_input(&raw);
    assert!(r.html.as_deref().unwrap().contains("<b>shared folder</b>"));
    assert!(!r.prefers_plain);
    assert!(mime::attachment(&raw, 0).is_none());
}

#[test]
fn pgp_signed_mail_hides_the_signature_and_shows_the_body() {
    let raw = signed("pgp-signed.eml");
    let body = mime::parse_body(&raw);
    assert!(body.attachments.is_empty(), "{:?}", body.attachments);
    assert!(body.text.starts_with("Hi Lara,"), "{}", body.text);
    assert!(body.text.contains("3f2a\u{2026}9c1d"));
    assert!(!body.text.contains("PGP SIGNATURE"));
    let r = mime::parse_render_input(&raw);
    assert_eq!(r.html, None);
    assert!(r.prefers_plain);
}

#[test]
fn signature_parts_outside_a_signed_message_stay_listed() {
    // Only the signature of a top-level multipart/signed is hidden: one
    // inside multipart/mixed is a file like any other.
    let raw = b"From: a@b.c\r\nContent-Type: multipart/mixed; boundary=M\r\n\r\n--M\r\nContent-Type: text/plain\r\n\r\nSee the key.\r\n--M\r\nContent-Type: application/pgp-signature; name=signature.asc\r\nContent-Disposition: attachment; filename=signature.asc\r\n\r\n-----BEGIN PGP SIGNATURE-----\r\n--M--\r\n";
    let names: Vec<_> = mime::parse_body(raw)
        .attachments
        .into_iter()
        .map(|a| a.name)
        .collect();
    assert_eq!(names, ["signature.asc"]);
}

#[test]
fn corpus_source_view_and_forward_as_attachment() {
    use komorebi::mail::{smtp, source};
    use mail_parser::PartType;
    for name in common::FILES {
        let raw = raw(name);
        // Show original: one line per field, nothing that hides or
        // reorders text.
        let original = source::original(&raw);
        assert!(original.headers.contains("\nSubject: "), "{name}");
        assert!(
            !original
                .headers
                .chars()
                .any(|c| (c.is_control() && c != '\n') || ('\u{202A}'..='\u{202E}').contains(&c)),
            "{name}"
        );
        // Forward as attachment: the nested message comes back byte for byte.
        let built = smtp::build_raw(smtp::Outgoing {
            from_email: "me@example.org".into(),
            to: "ana@example.org".into(),
            subject: "Fwd".into(),
            attachments: vec![smtp::OutAttachment {
                filename: source::attachment_name(name),
                data: raw.clone(),
            }],
            ..Default::default()
        })
        .unwrap();
        let msg = mail_parser::MessageParser::default().parse(&built).unwrap();
        let nested = msg
            .parts
            .iter()
            .find_map(|p| match &p.body {
                PartType::Message(m) => Some(m),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{name}: no attached message"));
        assert_eq!(nested.raw_message(), &raw[..], "{name}");
    }
}
