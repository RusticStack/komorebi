//! What the command line and a second launch can ask of the app: open a
//! composer prefilled from a `mailto:` URL (Windows runs
//! `komorebi.exe "mailto:..."` for a mail link when Komorebi is the chosen
//! mail app). Pure and tested; the Win32 side is in `src/win_shell.rs`.
//!
//! The URL only prefills a draft (`mail::render::links::mailto`: header
//! fields and body, nothing attached, controls stripped from headers). It
//! never sends anything, and any other argument is ignored quietly.

use crate::mail::render::links::{Mailto, mailto};

/// A `mailto:` URL longer than this is refused (Windows caps a command line
/// at 32,767 characters; this keeps the hand-off payload small too).
pub const MAX_URL_BYTES: usize = 16 * 1024;

/// Tag of the `WM_COPYDATA` message that carries a URL to the running copy
/// (`dwData`), so unrelated copy data is never read as one.
pub const COPYDATA_MAILTO: usize = 0x4B4D_4C54;

/// The first argument that is a usable `mailto:` URL, as given. Other
/// arguments (`--minimized`, files, other schemes) are skipped; an
/// over-long or NUL-holding one is refused.
pub fn mailto_url<I, S>(args: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .map(|a| a.as_ref().trim().to_string())
        .find(|a| usable(a))
}

fn usable(url: &str) -> bool {
    url.len() <= MAX_URL_BYTES && !url.contains('\0') && mailto(url).is_some()
}

/// The fields of the `mailto:` URL in the launch arguments, if any.
pub fn mailto_from_args<I, S>(args: I) -> Option<Mailto>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    mailto(&mailto_url(args)?)
}

/// What a second launch hands the running copy: the URL's bytes (UTF-8),
/// or nothing when the arguments hold no usable `mailto:` URL.
pub fn handoff_payload<I, S>(args: I) -> Option<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    mailto_url(args).map(String::into_bytes)
}

/// The fields a received hand-off carries: `tag` is the message's
/// `dwData`, `bytes` its payload. Wrong tag, too long, not UTF-8 or not a
/// `mailto:` URL: nothing.
pub fn decode_handoff(tag: usize, bytes: &[u8]) -> Option<Mailto> {
    if tag != COPYDATA_MAILTO || bytes.len() > MAX_URL_BYTES {
        return None;
    }
    let url = std::str::from_utf8(bytes).ok()?.trim_end_matches('\0');
    if !usable(url) {
        return None;
    }
    mailto(url)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_mailto_argument_among_others() {
        let m = mailto_from_args([
            "--minimized",
            "mailto:ana@example.org,rui@example.org?cc=c@example.org&subject=Hi%20there&body=a%0D%0Ab",
        ])
        .unwrap();
        assert_eq!(m.to, "ana@example.org,rui@example.org");
        assert_eq!(m.cc, "c@example.org");
        assert_eq!(m.subject, "Hi there");
        assert_eq!(m.body, "a\nb");
    }

    #[test]
    fn repeated_to_values_join() {
        let m = mailto_from_args(["MAILTO:a@x.org?to=b@x.org&to=c@x.org"]).unwrap();
        assert_eq!(m.to, "a@x.org, b@x.org, c@x.org");
    }

    #[test]
    fn other_arguments_are_refused_quietly() {
        for bad in [
            "--minimized",
            "",
            "https://example.com/",
            "file:///c:/x.txt",
            "C:/mail.eml",
            "javascript:alert(1)",
            "mailtox:a@b.c",
        ] {
            assert_eq!(mailto_from_args([bad]), None, "{bad}");
            assert_eq!(handoff_payload([bad]), None, "{bad}");
        }
        assert_eq!(mailto_from_args(Vec::<String>::new()), None);
    }

    #[test]
    fn hostile_urls_are_bounded_and_inert() {
        let long = format!("mailto:a@b.c?subject={}", "x".repeat(MAX_URL_BYTES));
        assert_eq!(mailto_from_args([long.as_str()]), None);
        assert_eq!(mailto_from_args(["mailto:a@b.c\0?subject=x"]), None);
        // Header injection and attachments from the URL do nothing.
        let m = mailto_from_args([
            "mailto:a@b.c?subject=Hi%0D%0ABcc:%20evil@x.org&attach=C:%5Csecret.txt&attachment=x&in-reply-to=1",
        ])
        .unwrap();
        assert!(!m.subject.contains('\n') && !m.subject.contains('\r'));
        assert_eq!(m.bcc, "");
        assert_eq!(m.to, "a@b.c");
        // Broken escapes and a huge body are capped, not fatal.
        let body = format!("mailto:?body={}%zz%", "y".repeat(MAX_URL_BYTES - 40));
        assert!(mailto_from_args([body.as_str()]).unwrap().body.len() <= 64 * 1024);
        assert!(mailto_from_args(["mailto:%ff%fe%00%"]).is_some());
    }

    #[test]
    fn handoff_round_trips_and_checks_the_message() {
        let args = ["--minimized", "mailto:ana@example.org?subject=Hello"];
        let payload = handoff_payload(args).unwrap();
        let m = decode_handoff(COPYDATA_MAILTO, &payload).unwrap();
        assert_eq!(m.to, "ana@example.org");
        assert_eq!(m.subject, "Hello");
        // A trailing NUL (a C string) is tolerated.
        let mut with_nul = payload.clone();
        with_nul.push(0);
        assert!(decode_handoff(COPYDATA_MAILTO, &with_nul).is_some());
        // Wrong tag, junk, oversize and non-UTF-8 are dropped.
        assert_eq!(decode_handoff(COPYDATA_MAILTO + 1, &payload), None);
        assert_eq!(decode_handoff(COPYDATA_MAILTO, b"https://x.org/"), None);
        assert_eq!(decode_handoff(COPYDATA_MAILTO, &[0xff, 0xfe, 0x6d]), None);
        assert_eq!(
            decode_handoff(COPYDATA_MAILTO, &vec![b'm'; MAX_URL_BYTES + 1]),
            None
        );
        assert_eq!(decode_handoff(COPYDATA_MAILTO, b""), None);
    }
}
