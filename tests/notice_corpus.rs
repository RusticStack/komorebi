//! The synthetic sender-notice, encrypted-mail and List-Unsubscribe
//! fixtures in `tests/fixtures/notice/`: each one's verdict as the reader
//! computes it on open (the account's IMAP host is `mail.komorebi.invalid`,
//! as in `examples/seed`).

use komorebi::mail::mime::{self, Encryption};
use komorebi::trust::{self, Reason};

const HOST: &str = "mail.komorebi.invalid";

fn raw(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/tests/fixtures/notice/{name}.eml",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn reasons(name: &str) -> Vec<Reason> {
    let raw = raw(name);
    let h = mime::parse_header(&raw);
    let body = mime::parse_body(&raw);
    trust::reasons(&body.auth, HOST, &h.from_name, &h.from_addr)
}

fn names(raw: &[u8]) -> Vec<String> {
    mime::parse_body(raw)
        .attachments
        .into_iter()
        .map(|a| a.name)
        .collect()
}

#[test]
fn sender_verdicts() {
    assert_eq!(reasons("01-dmarc-fail"), [Reason::Dmarc]);
    // The server's own topmost header passes; the lower ones are forged.
    assert_eq!(reasons("02-forged-lower-hop"), []);
    assert_eq!(
        reasons("03-display-name-spoof"),
        [Reason::NameDomain {
            shown: "paypal.com".into(),
            real: "mailer.example.net".into()
        }]
    );
    assert_eq!(
        reasons("04-punycode-sender"),
        [Reason::Punycode {
            ascii: "xn--80ak6aa92e.example".into(),
            unicode: "аррӏе.example".into()
        }]
    );
    assert_eq!(
        reasons("05-mixed-script-sender"),
        [Reason::MixedScript {
            domain: "bаnk.example".into(),
            script: "Cyrillic"
        }]
    );
    // No Authentication-Results and an ordinary sender: no notice.
    for name in [
        "06-smime-enveloped",
        "07-pgp-mime-encrypted",
        "08-pgp-inline",
        "09-unsubscribe-mailto",
        "10-unsubscribe-https",
    ] {
        assert_eq!(reasons(name), [], "{name}");
    }
    // Another account's server (another authserv-id domain): nothing.
    let raw = raw("01-dmarc-fail");
    let body = mime::parse_body(&raw);
    assert_eq!(body.auth.len(), 1);
    assert_eq!(trust::auth_verdict(&body.auth, "imap.example.org"), None);
}

#[test]
fn encrypted_mail_shows_no_ciphertext() {
    let cases = [
        ("06-smime-enveloped", Encryption::Smime, "smime.p7m"),
        (
            "07-pgp-mime-encrypted",
            Encryption::PgpMime,
            "encrypted.asc",
        ),
        ("08-pgp-inline", Encryption::PgpInline, "message.asc"),
    ];
    for (name, kind, file) in cases {
        let raw = raw(name);
        let body = mime::parse_body(&raw);
        assert_eq!(body.encrypted, Some(kind), "{name}");
        assert_eq!(body.preview(), "Encrypted message", "{name}");
        // The ciphertext is one file to save (the PGP/MIME version part is
        // not listed).
        assert_eq!(names(&raw), [file], "{name}");
        let (saved, data) = mime::attachment(&raw, 0).unwrap();
        assert_eq!(saved, file);
        assert!(!data.is_empty());
        assert_eq!(mime::attachments(&raw, |_| true).len(), 1);
        let input = mime::parse_render_input(&raw);
        assert!(input.prefers_plain && input.html.is_none(), "{name}");
        assert_eq!(input.text, body.text);
    }
    assert_eq!(mime::parse_body(&raw("06-smime-enveloped")).text, "");
    assert_eq!(mime::parse_body(&raw("07-pgp-mime-encrypted")).text, "");
    // Inline PGP keeps its text; the file is the armored block alone.
    let raw = raw("08-pgp-inline");
    assert!(mime::parse_body(&raw).text.starts_with("Hi,"));
    let (_, block) = mime::attachment(&raw, 0).unwrap();
    let block = String::from_utf8(block).unwrap();
    assert!(block.starts_with("-----BEGIN PGP MESSAGE-----\r\n"));
    assert!(block.ends_with("-----END PGP MESSAGE-----\r\n"));
    // Signed (not encrypted) mail is unchanged.
    let signed = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mime/pgp-signed.eml"
    ))
    .unwrap();
    let body = mime::parse_body(&signed);
    assert_eq!(body.encrypted, None);
    assert!(body.preview().starts_with("Hi Lara,"));
}

#[test]
fn unsubscribe_addresses() {
    // mailto is preferred over https, whatever the order.
    assert_eq!(
        mime::parse_body(&raw("09-unsubscribe-mailto")).unsubscribe,
        "mailto:unsubscribe@news.example.com?subject=unsubscribe%20abc123"
    );
    // Plain http is skipped; One-Click POST is not used.
    assert_eq!(
        mime::parse_body(&raw("10-unsubscribe-https")).unsubscribe,
        "https://shop.example.com/unsubscribe?u=42"
    );
    assert_eq!(mime::parse_body(&raw("01-dmarc-fail")).unsubscribe, "");
}
