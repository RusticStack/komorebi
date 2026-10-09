//! HTML replies to the synthetic corpus (`tests/fixtures/html`): the
//! original is sanitized, quoted under the reply's text, built into a
//! multipart/alternative message and rendered back as a recipient's
//! Komorebi would show it. Nothing remote survives the quote. Nothing here
//! touches the network.

mod common;

use common::{FILES, big_stack, raw};
use komorebi::mail::compose_html::{alternative, attach, reply_quote, sanitize};
use komorebi::mail::render::{self, Deadline, Engine, Opts};
use komorebi::mail::{mime, smtp};

const ATTRIBUTION: &str = "On Monday, Ana Silva wrote:";

/// The message a reply to corpus file `name` sends, as raw bytes.
fn reply_to(name: &str) -> Option<Vec<u8>> {
    let original = raw(name);
    let quoted = sanitize(&mime::html_body(&original)?)?;
    let text = mime::parse_body(&original).text;
    let quote: String = text.lines().map(|l| format!("> {l}\n")).collect();
    let body = format!("Thanks, **noted**.\n\n{ATTRIBUTION}\n{quote}");
    let stored = attach(&body, ATTRIBUTION, &reply_quote(ATTRIBUTION, &quoted));
    assert!(stored.is_some(), "{name}");
    let (body, body_html) = alternative(&body, stored.as_deref());
    let html = body_html.unwrap();
    assert!(html.contains("komorebi_quote"), "{name}");
    Some(
        smtp::build_raw(smtp::Outgoing {
            from_email: "me@example.com".into(),
            to: "Ana Silva <ana@example.com>".into(),
            subject: format!("Re: {name}"),
            body,
            body_html: Some(html),
            ..Default::default()
        })
        .unwrap(),
    )
}

#[test]
fn replies_to_the_corpus_quote_without_remote_content() {
    let mut quoted = 0;
    for name in FILES {
        let Some(reply) = reply_to(name) else {
            continue;
        };
        quoted += 1;
        let html = mime::html_body(&reply).unwrap().to_ascii_lowercase();
        for bad in [
            "<img",
            "<style",
            "<script",
            "<link",
            " src=\"",
            " background=\"",
        ] {
            assert!(!html.contains(bad), "{name}: {bad}");
        }
        // Text may say "url()"; no style attribute may use one.
        for style in html.split("style=\"").skip(1) {
            let value = style.split('"').next().unwrap();
            assert!(!value.contains("url("), "{name}: {value}");
        }
        let input = mime::parse_render_input(&reply);
        assert!(!input.prefers_plain, "{name}");
        let doc = big_stack(move || {
            let opts = Opts {
                width_px: 700.0,
                ..Default::default()
            };
            render::layout(&input, &opts, &mut Engine::new(), &Deadline::none()).unwrap()
        });
        assert_eq!(doc.blocked_remote, 0, "{name}");
        assert!(doc.remote_images.is_empty(), "{name}");
        let lines: Vec<String> = doc.text_lines().into_iter().map(|l| l.text).collect();
        assert!(lines.iter().any(|l| l.contains("Thanks, noted.")), "{name}");
        assert!(lines.iter().any(|l| l.contains(ATTRIBUTION)), "{name}");
    }
    // Every corpus file but the plain-preferring ones is HTML.
    assert!(quoted >= 12, "{quoted}");
}

/// The newsletter's blocked remote images (11 when read) are gone from the
/// quote, while its words stay.
#[test]
fn mailchimp_quote_keeps_text_and_drops_images() {
    let original = raw("01-mailchimp-campaign");
    let read = big_stack(move || {
        render::layout(
            &mime::parse_render_input(&original),
            &Opts::default(),
            &mut Engine::new(),
            &Deadline::none(),
        )
        .unwrap()
    });
    assert!(read.blocked_remote > 0);
    let reply = reply_to("01-mailchimp-campaign").unwrap();
    let text = mime::parse_body(&reply).text;
    assert!(text.starts_with("Thanks, **noted**."), "{text}");
    let doc = big_stack(move || {
        render::layout(
            &mime::parse_render_input(&reply),
            &Opts::default(),
            &mut Engine::new(),
            &Deadline::none(),
        )
        .unwrap()
    });
    assert_eq!(doc.blocked_remote, 0);
    let words: Vec<String> = read
        .text_lines()
        .into_iter()
        .map(|l| l.text)
        .filter(|l| l.len() > 20)
        .take(3)
        .collect();
    let quoted: String = doc
        .text_lines()
        .into_iter()
        .map(|l| l.text)
        .collect::<Vec<_>>()
        .join("\n");
    for w in &words {
        let first = w.split_whitespace().next().unwrap();
        assert!(quoted.contains(first), "{first} not in {quoted}");
    }
}
