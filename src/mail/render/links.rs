//! Link targets and what to warn about before one is opened. Only `http`,
//! `https` and `mailto` links, and `#fragment` links within the message,
//! are kept; every other scheme (`javascript:`, `vbscript:`, `data:`,
//! `file:`, `tel:`, relative paths) is inert.
//!
//! Web links are parsed and serialized by the `url` crate (WHATWG rules,
//! as browsers apply them), so the host checked here is the host a browser
//! would contact, in its `xn--` form, and the serialization handed to the
//! shell has no raw spaces or quotes.

use url::{Host, Url};

/// The followable form of an `href` (entities already decoded), or `None`
/// when it must stay inert. As in browsers, tabs and newlines are removed
/// anywhere and controls and spaces are trimmed from both ends first, so
/// `java&#9;script:` cannot sneak through.
pub fn sanitize(href: &str) -> Option<String> {
    let clean: String = href
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let clean = clean.trim_matches(|c: char| c <= ' ');
    if let Some(name) = clean.strip_prefix('#') {
        return (!name.is_empty()).then(|| clean.to_string());
    }
    let (scheme, _) = clean.split_once(':')?;
    if !["http", "https", "mailto"]
        .iter()
        .any(|s| scheme.eq_ignore_ascii_case(s))
    {
        return None;
    }
    let url = Url::parse(clean).ok()?;
    if url.scheme() == "mailto" && url.path().is_empty() && url.query().is_none() {
        return None;
    }
    Some(url.into())
}

/// The anchor name a `#fragment` link points at (percent-decoded).
pub fn fragment(href: &str) -> Option<String> {
    href.strip_prefix('#')
        .filter(|n| !n.is_empty())
        .map(percent_decode)
}

/// Reasons to look twice at a web link before opening it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Warning {
    /// The link text names a different site than the link goes to.
    TextMismatch,
    /// The host is an internationalized (punycode) name, which can imitate
    /// another site's letters.
    Punycode,
    /// The URL carries a user name (`https://bank.example@evil.example/`),
    /// which makes the real host easy to miss.
    UserInfo,
    /// The host is a raw IP address.
    RawIp,
}

/// A web link as the confirm prompt shows it: the URL split around its
/// host, so the host can stand out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebLink {
    /// The serialized URL: what is shown and what is opened.
    pub url: String,
    pub before: String,
    /// In its `xn--` form for internationalized names.
    pub host: String,
    pub after: String,
    pub warnings: Vec<Warning>,
}

/// Inspect an http(s) link whose visible text is `text`; `None` for any
/// other scheme.
pub fn web(href: &str, text: &str) -> Option<WebLink> {
    let url = Url::parse(href).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_string();
    let mut warnings = vec![];
    if let Some(shown) = text_host(text)
        && !registrable(&shown).eq_ignore_ascii_case(registrable(&host))
    {
        warnings.push(Warning::TextMismatch);
    }
    match url.host() {
        Some(Host::Domain(d)) if d.split('.').any(|l| l.starts_with("xn--")) => {
            warnings.push(Warning::Punycode);
        }
        Some(Host::Ipv4(_) | Host::Ipv6(_)) => warnings.push(Warning::RawIp),
        _ => {}
    }
    if !url.username().is_empty() || url.password().is_some() {
        warnings.push(Warning::UserInfo);
    }
    let s = String::from(url);
    // The host follows the scheme's "//" and any "userinfo@".
    let auth = s.find("//").map_or(0, |i| i.saturating_add(2));
    let auth_end = s
        .get(auth..)
        .and_then(|a| a.find(['/', '?', '#']))
        .map_or(s.len(), |i| i.saturating_add(auth));
    let start = s
        .get(auth..auth_end)
        .and_then(|a| a.rfind('@'))
        .map_or(auth, |i| i.saturating_add(auth).saturating_add(1));
    let end = start.saturating_add(host.len());
    let (before, after) = match (s.get(..start), s.get(end..)) {
        (Some(b), Some(a)) if s.get(start..end) == Some(host.as_str()) => (b.into(), a.into()),
        _ => (String::new(), String::new()),
    };
    Some(WebLink {
        url: s,
        before,
        host,
        after,
        warnings,
    })
}

/// The host named by link text that reads as a URL or a domain
/// ("https://bank.example/login", "www.bank.example"), in the form `Url`
/// gives hosts. `None` for ordinary words.
fn text_host(text: &str) -> Option<String> {
    let t = text.trim().trim_matches(|c: char| {
        matches!(
            c,
            '<' | '>' | '(' | ')' | '[' | ']' | '"' | '\'' | '.' | ',' | ';' | ':'
        )
    });
    if t.is_empty() || t.chars().any(char::is_whitespace) {
        return None;
    }
    let lower = t.to_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Url::parse(t).ok()?.host_str().map(str::to_string);
    }
    let host = t.split(['/', '?', '#']).next()?;
    let host = match host.rsplit_once(':') {
        Some((h, port)) if port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => host,
    };
    let labels: Vec<&str> = host.split('.').collect();
    let tld = labels.last()?.to_lowercase();
    // A file name is not a domain, even though it reads like one.
    const FILES: [&str; 14] = [
        "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "png", "jpg", "jpeg", "gif", "txt",
        "csv", "ics",
    ];
    let shaped = labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_alphanumeric() || c == '-'))
        && (tld.starts_with("xn--")
            || tld.chars().count() >= 2 && tld.chars().all(char::is_alphabetic))
        && !FILES.contains(&tld.as_str());
    if !shaped {
        return None;
    }
    Url::parse(&format!("http://{host}/"))
        .ok()?
        .host_str()
        .map(str::to_string)
}

/// The part of `host` a site owner registers: the last two labels, or the
/// last three under a generic second level of a country code (`co.uk`,
/// `com.br`). A heuristic, not the public suffix list; IP addresses are
/// kept whole. Shared with the sender notice (`crate::trust`).
pub fn registrable(host: &str) -> &str {
    let host = host.trim_end_matches('.');
    if host.starts_with('[') || host.parse::<std::net::Ipv4Addr>().is_ok() {
        return host;
    }
    let mut labels = host.rsplit('.');
    let (tld, sld) = (labels.next().unwrap_or(""), labels.next().unwrap_or(""));
    const GENERIC: [&str; 12] = [
        "co", "com", "net", "org", "gov", "edu", "ac", "or", "ne", "go", "gob", "mil",
    ];
    // Dots inside the registrable part: one, or two under `co.uk`.
    let dots = if tld.len() == 2 && GENERIC.contains(&sld) {
        2
    } else {
        1
    };
    let start = host
        .char_indices()
        .rev()
        .filter(|&(_, c)| c == '.')
        .nth(dots)
        .map_or(0, |(i, _)| i.saturating_add(1));
    host.get(start..).unwrap_or(host)
}

/// The fields of a `mailto:` link (RFC 6068), for the composer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mailto {
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
}

/// Prefilled header fields are capped at this many characters, the body at
/// [`MAX_MAILTO_BODY`].
const MAX_MAILTO_FIELD: usize = 2000;
const MAX_MAILTO_BODY: usize = 64 * 1024;

/// Parse a `mailto:` link. Only `to`, `cc`, `bcc`, `subject` and `body` are
/// taken; any other header the link names is ignored. Unlike form data, a
/// `+` stays a plus.
pub fn mailto(href: &str) -> Option<Mailto> {
    let (scheme, rest) = href.split_once(':')?;
    if !scheme.eq_ignore_ascii_case("mailto") {
        return None;
    }
    let rest = rest.split('#').next().unwrap_or("");
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut m = Mailto {
        to: percent_decode(path),
        ..Default::default()
    };
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = percent_decode(v);
        match percent_decode(k).to_ascii_lowercase().as_str() {
            "to" => append(&mut m.to, v),
            "cc" => append(&mut m.cc, v),
            "bcc" => append(&mut m.bcc, v),
            "subject" => m.subject = v,
            "body" => m.body = v,
            _ => {}
        }
    }
    let field = |s: &str| -> String {
        let one_line: String = s
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(MAX_MAILTO_FIELD)
            .collect();
        one_line.trim().to_string()
    };
    Some(Mailto {
        to: field(&m.to),
        cc: field(&m.cc),
        bcc: field(&m.bcc),
        subject: field(&m.subject),
        body: m
            .body
            .replace("\r\n", "\n")
            .chars()
            .take(MAX_MAILTO_BODY)
            .collect(),
    })
}

fn append(list: &mut String, more: String) {
    if more.trim().is_empty() {
        return;
    }
    if !list.trim().is_empty() {
        list.push_str(", ");
    }
    list.push_str(&more);
}

/// `%XX` escapes decoded as UTF-8 (invalid sequences become U+FFFD).
fn percent_decode(s: &str) -> String {
    let hex = |b: u8| (b as char).to_digit(16).and_then(|d| u8::try_from(d).ok());
    let mut out = Vec::with_capacity(s.len());
    let mut bytes = s.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let mut ahead = bytes.clone();
            if let (Some(hi), Some(lo)) = (ahead.next().and_then(hex), ahead.next().and_then(hex)) {
                out.push(hi.wrapping_shl(4) | lo);
                bytes = ahead;
                continue;
            }
        }
        out.push(b);
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn only_web_and_mail_links_survive() {
        assert_eq!(
            sanitize(" https://example.com/a b\n").as_deref(),
            Some("https://example.com/a%20b")
        );
        assert_eq!(
            sanitize("HTTPS://Example.COM").as_deref(),
            Some("https://example.com/")
        );
        assert!(sanitize("mailto:a@example.com").is_some());
        assert!(sanitize("#fn1").is_some());
        for bad in [
            "javascript:alert(1)",
            "java\tscript:alert(1)",
            " JaVaScRiPt:x",
            "vbscript:x",
            "data:text/html,<b>",
            "file:///c:/x",
            "tel:+351",
            "cid:not-a-link",
            "relative/path",
            "#",
            "https:",
            "mailto:",
        ] {
            assert_eq!(sanitize(bad), None, "{bad}");
        }
    }

    /// The obfuscated script links of corpus 14, as the DOM hands them over
    /// (entities decoded).
    #[test]
    fn obfuscated_script_links_from_14_stay_inert() {
        let eml = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/html/14-adversarial-malformed.eml"
        ))
        .unwrap();
        let html = String::from_utf8_lossy(&eml);
        for raw in [
            "JaVaScRiPt&#58;alert(2)",
            "  java&#x09;script:alert(3)",
            "javascript:alert('js-href')",
        ] {
            assert!(html.contains(&format!("href=\"{raw}\"")), "{raw} not in 14");
            let decoded = crate::mail::html::decode_entities(raw);
            assert!(decoded.to_ascii_lowercase().contains("script"));
            assert_eq!(sanitize(&decoded), None, "{decoded:?}");
        }
        // A newline or NUL inside the scheme does not help either.
        assert_eq!(sanitize("java\nscript:alert(1)"), None);
        assert_eq!(sanitize("\u{0}javascript:alert(1)"), None);
        assert_eq!(sanitize("java\u{0}script:alert(1)"), None);
    }

    fn warnings(href: &str, text: &str) -> Vec<Warning> {
        web(&sanitize(href).unwrap(), text).unwrap().warnings
    }

    #[test]
    fn text_naming_another_site_warns() {
        use Warning::*;
        let w = |text| warnings("https://evil.example.invalid/login", text);
        assert_eq!(w("https://bank.example.com/login"), [TextMismatch]);
        assert_eq!(w("www.bank.example.com"), [TextMismatch]);
        assert_eq!(w("bank.example.com."), [TextMismatch]);
        // Ordinary words, file names and addresses are not URLs.
        for text in [
            "Verify account",
            "Click here",
            "statement.pdf",
            "a@b.example",
            "",
        ] {
            assert_eq!(w(text), [], "{text}");
        }
        // The same site under another name or path is fine.
        let ok = |href, text| assert_eq!(warnings(href, text), [], "{href} {text}");
        ok("https://www.example.com/x?y=1", "example.com");
        ok("https://mail.example.com/", "https://example.com/inbox");
        ok("https://shop.example.co.uk/", "example.co.uk");
        ok("https://example.com/", "EXAMPLE.COM");
        // A different registrant under the same country suffix is not.
        assert_eq!(
            warnings("https://evil.co.uk/", "bank.co.uk"),
            [TextMismatch]
        );
        // The text is compared in punycode form too.
        assert_eq!(
            warnings("https://xn--bcher-kva.example/", "bücher.example"),
            [Punycode]
        );
    }

    #[test]
    fn punycode_hosts_warn_and_show_the_xn_form() {
        let l = web(
            &sanitize("https://xn--pypal-4ve.example.invalid/").unwrap(),
            "IDN homograph",
        )
        .unwrap();
        assert_eq!(l.warnings, [Warning::Punycode]);
        assert_eq!(l.host, "xn--pypal-4ve.example.invalid");
        // A Unicode host is converted to the form the browser will use.
        let l = web(&sanitize("https://pаypal.example/").unwrap(), "").unwrap();
        assert!(l.host.starts_with("xn--"), "{}", l.host);
        assert_eq!(l.warnings, [Warning::Punycode]);
    }

    #[test]
    fn userinfo_warns_and_the_real_host_is_split_out() {
        let l = web(
            &sanitize("https://bank.example.com@evil.example.invalid/x?q#f").unwrap(),
            "userinfo URL",
        )
        .unwrap();
        assert_eq!(l.warnings, [Warning::UserInfo]);
        assert_eq!(
            (l.before.as_str(), l.host.as_str(), l.after.as_str()),
            (
                "https://bank.example.com@",
                "evil.example.invalid",
                "/x?q#f"
            )
        );
        assert_eq!(format!("{}{}{}", l.before, l.host, l.after), l.url);
        assert_eq!(
            warnings("https://user:pw@example.com/", ""),
            [Warning::UserInfo]
        );
    }

    #[test]
    fn raw_ip_hosts_warn() {
        for href in [
            "http://192.168.1.10/login",
            "http://3232235786/",
            "http://0xC0.0xA8.1.10/",
            "https://[2001:db8::1]:8443/",
        ] {
            assert_eq!(warnings(href, ""), [Warning::RawIp], "{href}");
        }
        let l = web("http://3232235786/", "").unwrap();
        assert_eq!(l.host, "192.168.1.10");
        assert_eq!(warnings("https://example.com/", "Pay now"), []);
        assert!(web("mailto:a@example.com", "").is_none());
    }

    #[test]
    fn mailto_fields_are_parsed() {
        let m = mailto(
            &sanitize(
                "mailto:ana@example.com,%20rui@example.com?cc=CC@example.com&BCC=b@example.com\
                 &subject=Ol%C3%A1%20a+b&body=Line%201%0D%0ALine%202&to=extra@example.com\
                 &from=spoof@example.com&attach=C:%5Cx",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            m,
            Mailto {
                to: "ana@example.com, rui@example.com, extra@example.com".into(),
                cc: "CC@example.com".into(),
                bcc: "b@example.com".into(),
                subject: "Olá a+b".into(),
                body: "Line 1\nLine 2".into(),
            }
        );
        // Literal spaces survive sanitizing (as %20), and header fields
        // stay on one line.
        let m = mailto(&sanitize("mailto:x@example.com?subject=Hi there%0AX-Evil: 1").unwrap())
            .unwrap();
        assert_eq!(m.subject, "Hi there X-Evil: 1");
        assert_eq!(mailto("mailto:?subject=x").unwrap().to, "");
        assert_eq!(mailto("https://example.com/"), None);
        // Broken escapes are kept as typed.
        assert_eq!(
            mailto("mailto:a@b?subject=100%25%zz%").unwrap().subject,
            "100%%zz%"
        );
    }

    #[test]
    fn fragments_decode() {
        assert_eq!(fragment("#sec%201").as_deref(), Some("sec 1"));
        assert_eq!(fragment("#"), None);
        assert_eq!(fragment("https://x/#a"), None);
    }
}
