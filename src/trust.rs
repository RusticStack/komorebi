//! Reading cues about who sent a message, computed when it is opened from
//! the cached raw message: a failed sender check recorded by the account's
//! own server, a display name that names another domain, and a sender
//! domain that is an international (`xn--`) name or mixes scripts. Pure
//! functions over untrusted header text; nothing here verifies anything or
//! touches the network.
//!
//! Known limits: the registrable domain is a heuristic (the last two
//! labels, or three under `co.uk`, `com.br`, `com.pt` and the like), not
//! the public suffix list, and an `Authentication-Results` header is
//! trusted when its authserv-id has the IMAP server's registrable domain
//! (RFC 8601 servers remove such headers that arrive from outside).

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic
)]

use crate::mail::render::links::registrable;
use url::Url;

/// `Authentication-Results` headers looked at, and characters read of each.
pub const MAX_AUTH_HEADERS: usize = 10;
pub const MAX_AUTH_LEN: usize = 8 * 1024;
/// Display-name tokens looked at.
const MAX_NAME_TOKENS: usize = 32;

/// Why a sender deserves a second look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// The account's server recorded `dmarc=fail`.
    Dmarc,
    /// Both SPF and DKIM failed and nothing passed.
    SpfDkim,
    /// The display name names another domain than the address has.
    NameDomain { shown: String, real: String },
    /// The sender domain is an international name (shown decoded).
    Punycode { ascii: String, unicode: String },
    /// A label of the sender domain mixes Latin with another script.
    MixedScript {
        domain: String,
        script: &'static str,
    },
}

impl Reason {
    fn text(&self) -> String {
        match self {
            Reason::Dmarc => "it failed your server's sender check (DMARC)".into(),
            Reason::SpfDkim => "it failed your server's sender checks (SPF and DKIM)".into(),
            Reason::NameDomain { shown, real } => {
                format!("the name shows {shown}, but the address is at {real}")
            }
            Reason::Punycode { ascii, unicode } => {
                format!("{ascii} is the international name \u{201c}{unicode}\u{201d}")
            }
            Reason::MixedScript { domain, script } => {
                format!("{domain} mixes Latin and {script} letters")
            }
        }
    }
}

/// The reasons to look twice at a message from `name <addr>`, received by
/// an account on `imap_host`, given its `Authentication-Results` headers
/// (topmost first). At most two, the server's verdict first.
pub fn reasons(auth: &[String], imap_host: &str, name: &str, addr: &str) -> Vec<Reason> {
    let mut out: Vec<Reason> = auth_verdict(auth, imap_host).into_iter().collect();
    let domain = addr
        .rsplit_once('@')
        .map(|(_, d)| d.trim().trim_end_matches('.').to_lowercase())
        .unwrap_or_default();
    if let Some(real) = host(&domain) {
        if let Some(shown) = name_domain(name, &real) {
            out.push(Reason::NameDomain {
                shown,
                real: real.clone(),
            });
        }
        let unicode = if domain.split('.').any(|l| l.starts_with("xn--")) {
            let unicode = to_unicode(&real);
            if unicode != real {
                out.push(Reason::Punycode {
                    ascii: real.clone(),
                    unicode: unicode.clone(),
                });
            }
            unicode
        } else {
            domain.clone()
        };
        if let Some(script) = unicode.split('.').find_map(mixed_script) {
            out.push(Reason::MixedScript {
                domain: unicode,
                script,
            });
        }
    }
    out.truncate(2);
    out
}

/// The notice line for [`reasons`], or an empty string.
pub fn notice(auth: &[String], imap_host: &str, name: &str, addr: &str) -> String {
    let reasons = reasons(auth, imap_host, name, addr);
    if reasons.is_empty() {
        return String::new();
    }
    let text: Vec<String> = reasons.iter().map(Reason::text).collect();
    format!("Be careful with this sender: {}.", text.join("; "))
}

/// The verdict of the topmost `Authentication-Results` header added by the
/// account's own server (its authserv-id has the IMAP host's registrable
/// domain); headers further down, which the sender could have written, and
/// those of other servers are ignored. `None` when no header is the
/// server's or nothing failed.
pub fn auth_verdict(auth: &[String], imap_host: &str) -> Option<Reason> {
    let mine = host(imap_host.trim())?;
    let mine = registrable(&mine);
    let (_, results) = auth
        .iter()
        .take(MAX_AUTH_HEADERS)
        .filter_map(|h| parse_auth(h))
        .find(|(id, _)| host(id).is_some_and(|h| registrable(&h) == mine))?;
    let has = |method: &str, verdicts: &[&str]| {
        results
            .iter()
            .any(|(m, r)| m == method && verdicts.contains(&r.as_str()))
    };
    if has("dmarc", &["fail"]) {
        return Some(Reason::Dmarc);
    }
    let passed = has("dmarc", &["pass"]) || has("spf", &["pass"]) || has("dkim", &["pass"]);
    (!passed && has("spf", &["fail", "softfail"]) && has("dkim", &["fail"]))
        .then_some(Reason::SpfDkim)
}

/// An `Authentication-Results` value (RFC 8601): the authserv-id and each
/// `method=result`, lower-cased, comments removed. `None` without an id.
pub fn parse_auth(value: &str) -> Option<(String, Vec<(String, String)>)> {
    let clean = strip_comments(value);
    let mut parts = clean.split(';');
    let id = parts.next()?.split_whitespace().next()?.to_lowercase();
    let results = parts
        .filter_map(|p| {
            let (method, result) = p.split_whitespace().next()?.split_once('=')?;
            let method = method.split('/').next().unwrap_or(method);
            Some((method.to_lowercase(), result.to_lowercase()))
        })
        .collect();
    Some((id, results))
}

/// `value` (at most [`MAX_AUTH_LEN`] characters) without `(comments)`,
/// which nest; quoted strings are kept as they are.
fn strip_comments(value: &str) -> String {
    let mut out = String::with_capacity(value.len().min(MAX_AUTH_LEN));
    let (mut depth, mut quoted, mut escaped) = (0u32, false, false);
    for c in value.chars().take(MAX_AUTH_LEN) {
        if escaped {
            escaped = false;
            if depth == 0 {
                out.push(c);
            }
            continue;
        }
        match c {
            '\\' => escaped = true,
            '"' if depth == 0 => quoted = !quoted,
            '(' if !quoted => depth = depth.saturating_add(1),
            ')' if !quoted && depth > 0 => {
                depth = depth.saturating_sub(1);
                continue;
            }
            _ => {}
        }
        if depth == 0 {
            out.push(c);
        }
    }
    out
}

/// A domain as a browser would contact it (lower case, `xn--` form), or
/// `None` if it doesn't read as a host name.
fn host(domain: &str) -> Option<String> {
    let domain = domain.trim().trim_end_matches('.');
    if domain.is_empty() || domain.contains(['/', '?', '#', '@', ':', '\\', ' ']) {
        return None;
    }
    Url::parse(&format!("http://{domain}/"))
        .ok()?
        .host_str()
        .map(str::to_string)
}

/// A domain the display name shows (in an address, or bare with a common
/// top-level domain, `www.` or a scheme) whose registrable part differs
/// from the sender's `real` host.
fn name_domain(name: &str, real: &str) -> Option<String> {
    const TLDS: [&str; 24] = [
        "com", "net", "org", "io", "co", "pt", "br", "uk", "de", "fr", "es", "it", "nl", "eu",
        "info", "biz", "us", "ca", "au", "me", "app", "dev", "gov", "edu",
    ];
    let real = registrable(real);
    name.split(|c: char| c.is_whitespace() || "<>()[]\"',;".contains(c))
        .filter(|t| !t.is_empty())
        .take(MAX_NAME_TOKENS)
        .find_map(|token| {
            let lower = token.to_lowercase();
            let (candidate, explicit) = match lower.rsplit_once('@') {
                Some((_, d)) => (d, true),
                None => {
                    let rest = lower
                        .strip_prefix("https://")
                        .or_else(|| lower.strip_prefix("http://"));
                    let d = rest.unwrap_or(&lower);
                    let d = d.split(['/', '?', '#']).next().unwrap_or(d);
                    (d, rest.is_some() || d.starts_with("www."))
                }
            };
            let candidate = candidate.trim_end_matches(['.', ':', '!', '?']);
            let labels: Vec<&str> = candidate.split('.').collect();
            let shaped = labels.len() >= 2
                && labels
                    .iter()
                    .all(|l| !l.is_empty() && l.chars().all(|c| c.is_alphanumeric() || c == '-'));
            let tld = labels.last().copied().unwrap_or_default();
            if !shaped || !(explicit || TLDS.contains(&tld)) {
                return None;
            }
            let shown = host(candidate)?;
            (registrable(&shown) != real).then_some(shown)
        })
}

/// `host` with its `xn--` labels decoded (RFC 3492); a label that doesn't
/// decode stays as it is.
fn to_unicode(host: &str) -> String {
    let labels: Vec<String> = host
        .split('.')
        .map(|l| {
            l.strip_prefix("xn--")
                .and_then(idna::punycode::decode)
                .map_or_else(|| l.to_string(), |c| c.into_iter().collect())
        })
        .collect();
    labels.join(".")
}

/// The script a domain label mixes with Latin letters, if any.
fn mixed_script(label: &str) -> Option<&'static str> {
    let (mut latin, mut cyrillic, mut greek) = (false, false, false);
    for c in label.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '\u{00C0}'..='\u{024F}' | '\u{1E00}'..='\u{1EFF}' => {
                latin |= c != '\u{00D7}' && c != '\u{00F7}';
            }
            '\u{0400}'..='\u{052F}'
            | '\u{1C80}'..='\u{1C8F}'
            | '\u{2DE0}'..='\u{2DFF}'
            | '\u{A640}'..='\u{A69F}' => cyrillic = true,
            '\u{0370}'..='\u{03FF}' | '\u{1F00}'..='\u{1FFF}' => greek = true,
            _ => {}
        }
    }
    match (latin, cyrillic, greek) {
        (true, true, _) => Some("Cyrillic"),
        (true, false, true) => Some("Greek"),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    const HOST: &str = "imap.mail.example.net";

    fn h(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_authentication_results() {
        let (id, r) = parse_auth(
            " mx1.example.net 1; spf=pass (sender ok; x=y) smtp.mailfrom=a@b.c;\r\n\
             \tdkim=fail (bad sig) header.d=b.c; DMARC=Fail (p=REJECT) header.from=b.c",
        )
        .unwrap();
        assert_eq!(id, "mx1.example.net");
        assert_eq!(
            r,
            [
                ("spf".to_string(), "pass".to_string()),
                ("dkim".into(), "fail".into()),
                ("dmarc".into(), "fail".into())
            ]
        );
        let (id, r) = parse_auth("example.net; none").unwrap();
        assert_eq!((id.as_str(), r.len()), ("example.net", 0));
        // A quoted reason with parentheses and a version on the method.
        let (_, r) =
            parse_auth("x.example.net; dkim/1=pass reason=\"ok (really)\"; spf=none").unwrap();
        assert_eq!(r[0], ("dkim".into(), "pass".into()));
        assert_eq!(r[1], ("spf".into(), "none".into()));
        assert_eq!(parse_auth(""), None);
        assert_eq!(parse_auth("(only a comment)"), None);
    }

    #[test]
    fn only_the_servers_topmost_header_counts() {
        let fail = "mx.example.net; spf=pass; dkim=pass; dmarc=fail header.from=bank.example";
        let pass = "mx.example.net; spf=pass; dkim=pass; dmarc=pass";
        assert_eq!(auth_verdict(&h(&[fail]), HOST), Some(Reason::Dmarc));
        // A forged lower-hop header (written by the sender) is ignored...
        assert_eq!(auth_verdict(&h(&[pass, fail]), HOST), None);
        assert_eq!(auth_verdict(&h(&[fail, pass]), HOST), Some(Reason::Dmarc));
        // ...as is one naming another server, wherever it is.
        let other = "mx.evil.example; dmarc=fail";
        assert_eq!(auth_verdict(&h(&[other, pass]), HOST), None);
        assert_eq!(auth_verdict(&h(&[other]), HOST), None);
        assert_eq!(auth_verdict(&[], HOST), None);
        // The registrable part is compared, under country suffixes too.
        assert_eq!(
            auth_verdict(&h(&["example.net; dmarc=fail"]), HOST),
            Some(Reason::Dmarc)
        );
        assert_eq!(
            auth_verdict(&h(&["mx.other.co.uk; dmarc=fail"]), "imap.mail.co.uk"),
            None
        );
    }

    #[test]
    fn spf_and_dkim_must_both_fail_with_nothing_passing() {
        let v = |r: &str| auth_verdict(&h(&[&format!("mx.example.net; {r}")]), HOST);
        assert_eq!(v("spf=fail; dkim=fail"), Some(Reason::SpfDkim));
        assert_eq!(
            v("spf=softfail; dkim=fail; dmarc=none"),
            Some(Reason::SpfDkim)
        );
        assert_eq!(v("spf=fail; dkim=fail; dkim=pass"), None);
        assert_eq!(v("spf=fail; dkim=none"), None);
        assert_eq!(v("spf=fail"), None);
        assert_eq!(v("spf=fail; dkim=fail; dmarc=pass"), None);
        assert_eq!(v("spf=pass; dkim=pass; dmarc=pass"), None);
    }

    #[test]
    fn display_names_naming_another_domain() {
        let r = |name: &str, addr: &str| reasons(&[], HOST, name, addr);
        assert_eq!(
            r("PayPal.com Support", "help@evil.example"),
            [Reason::NameDomain {
                shown: "paypal.com".into(),
                real: "evil.example".into()
            }]
        );
        assert_eq!(
            r("\"service@paypal.com\"", "x@mailer.evil.example")[0],
            Reason::NameDomain {
                shown: "paypal.com".into(),
                real: "mailer.evil.example".into()
            }
        );
        assert!(matches!(
            r("www.bank.example (Security)", "a@b.example")[0],
            Reason::NameDomain { .. }
        ));
        // The same site, ordinary names and dotted words are fine.
        for (name, addr) in [
            ("news@shop.example.com", "bounce@mail.example.com"),
            ("Shop.co.uk", "hello@news.shop.co.uk"),
            ("Dr. Smith", "smith@clinic.example"),
            ("A.B. Smith", "ab@x.example"),
            ("John.Doe", "jd@x.example"),
            ("e.g. Ana", "ana@x.example"),
            ("Ana Silva", "ana@x.example"),
            ("", "ana@x.example"),
            ("Report.pdf", "ana@x.example"),
        ] {
            assert_eq!(r(name, addr), [], "{name} <{addr}>");
        }
    }

    #[test]
    fn international_and_mixed_script_domains() {
        let r = reasons(&[], HOST, "PayPal", "service@xn--pypal-4ve.com");
        assert_eq!(
            r,
            [
                Reason::Punycode {
                    ascii: "xn--pypal-4ve.com".into(),
                    unicode: "pаypal.com".into()
                },
                Reason::MixedScript {
                    domain: "pаypal.com".into(),
                    script: "Cyrillic"
                }
            ]
        );
        // A raw Unicode domain (as a parser may hand it over).
        assert_eq!(
            reasons(&[], HOST, "", "a@αpple.example"),
            [Reason::MixedScript {
                domain: "αpple.example".into(),
                script: "Greek"
            }]
        );
        // An all-Cyrillic or accented Latin domain is not mixed.
        assert_eq!(
            reasons(&[], HOST, "", "a@xn--80ak6aa92e.com"),
            [Reason::Punycode {
                ascii: "xn--80ak6aa92e.com".into(),
                unicode: "аррӏе.com".into()
            }]
        );
        assert_eq!(mixed_script("café"), None);
        assert_eq!(mixed_script("bücher"), None);
        assert_eq!(reasons(&[], HOST, "", "a@xn--.com"), []);
    }

    #[test]
    fn notice_lists_at_most_two_reasons() {
        let auth = h(&["mx.example.net; dmarc=fail"]);
        assert_eq!(
            notice(&auth, HOST, "PayPal", "service@xn--pypal-4ve.com"),
            "Be careful with this sender: it failed your server's sender check (DMARC); \
             xn--pypal-4ve.com is the international name \u{201c}pаypal.com\u{201d}."
        );
        assert_eq!(notice(&[], HOST, "Ana", "ana@example.com"), "");
        assert_eq!(notice(&auth, "", "Ana", "ana@example.com"), "");
    }

    #[test]
    fn hostile_input_never_panics() {
        let long = "(".repeat(20_000) + &")".repeat(5);
        for v in [
            long.as_str(),
            "\\",
            "\"",
            ";;;;",
            "a; =; ==; spf=",
            "é; dmarc=fail",
        ] {
            parse_auth(v);
            auth_verdict(&[v.to_string()], HOST);
        }
        for addr in [
            "",
            "@",
            "a@",
            "a@.",
            "a@[1.2.3.4]",
            "a@xn--",
            "a@xn--zz.xn--",
        ] {
            reasons(&[], HOST, "x@y@z.com..", addr);
        }
        reasons(&[], HOST, &"a.com ".repeat(10_000), "a@b.example");
    }
}
