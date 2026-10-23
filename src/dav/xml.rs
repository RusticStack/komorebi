//! WebDAV XML: the fixed request bodies, and the multistatus reader
//! (RFC 4918, RFC 6578 sync-collection, RFC 6352 CardDAV, RFC 4791
//! CalDAV) on `roxmltree` with namespace-aware matching, no DTD, and caps
//! on nodes and responses. Hrefs are resolved against the request URL.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic
)]

use url::Url;

pub const DAV: &str = "DAV:";
pub const CARDDAV: &str = "urn:ietf:params:xml:ns:carddav";
pub const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
pub const CS: &str = "http://calendarserver.org/ns/";

/// XML nodes read at most per response.
pub const MAX_NODES: u32 = 200_000;
/// `<response>` elements read at most per multistatus.
pub const MAX_RESPONSES: usize = 5_000;

pub const PROPFIND_PRINCIPAL: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/><d:resourcetype/></d:prop></d:propfind>"#;

pub const PROPFIND_HOMES: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav" xmlns:cal="urn:ietf:params:xml:ns:caldav"><d:prop><card:addressbook-home-set/><cal:calendar-home-set/></d:prop></d:propfind>"#;

pub const PROPFIND_COLLECTIONS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav" xmlns:cal="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/"><d:prop><d:resourcetype/><d:displayname/><d:supported-report-set/><cs:getctag/><d:sync-token/><card:supported-address-data/><cal:supported-calendar-component-set/></d:prop></d:propfind>"#;

pub const PROPFIND_CTAG: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/"><d:prop><cs:getctag/><d:sync-token/></d:prop></d:propfind>"#;

pub const PROPFIND_ETAGS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:getetag/><d:resourcetype/></d:prop></d:propfind>"#;

/// sync-collection with `token` (empty the first time).
pub fn sync_collection(token: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<d:sync-collection xmlns:d="DAV:"><d:sync-token>{}</d:sync-token><d:sync-level>1</d:sync-level><d:prop><d:getetag/></d:prop></d:sync-collection>"#,
        escape(token)
    )
}

/// addressbook-multiget (`card`) or calendar-multiget for `hrefs`.
pub fn multiget(card: bool, hrefs: &[String]) -> String {
    let (ns, report, data) = if card {
        (CARDDAV, "addressbook-multiget", "address-data")
    } else {
        (CALDAV, "calendar-multiget", "calendar-data")
    };
    let mut out = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<x:{report} xmlns:d="DAV:" xmlns:x="{ns}"><d:prop><d:getetag/><x:{data}/></d:prop>"#
    );
    for h in hrefs {
        out.push_str("<d:href>");
        out.push_str(&escape(h));
        out.push_str("</d:href>");
    }
    out.push_str(&format!("</x:{report}>"));
    out
}

/// `s` with the five XML specials escaped.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// One `<response>`: its href (a path on the server, as sent) and what
/// this client reads from its 200 propstat.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Item {
    pub href: String,
    /// The response's own status (404: removed, in sync-collection), else
    /// 200.
    pub status: u16,
    pub etag: Option<String>,
    pub displayname: Option<String>,
    pub ctag: Option<String>,
    pub sync_token: Option<String>,
    pub collection: bool,
    pub addressbook: bool,
    pub calendar: bool,
    pub sync_collection: bool,
    pub vcard4: bool,
    /// Calendar components offered (`VEVENT`, `VTODO`); none listed: unknown.
    pub components: Vec<String>,
    pub principal: Option<String>,
    pub addressbook_home: Option<String>,
    pub calendar_home: Option<String>,
    /// address-data or calendar-data.
    pub data: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Multistatus {
    pub items: Vec<Item>,
    /// The multistatus-level sync-token (sync-collection).
    pub sync_token: Option<String>,
    /// A 507 on the request URL: more changes wait (truncated).
    pub truncated: bool,
}

fn status_code(text: &str) -> u16 {
    text.split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0)
}

fn is(node: roxmltree::Node, ns: &str, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name && node.tag_name().namespace() == Some(ns)
}

fn child<'a, 'i>(
    node: roxmltree::Node<'a, 'i>,
    ns: &str,
    name: &str,
) -> Option<roxmltree::Node<'a, 'i>> {
    node.children().find(|c| is(*c, ns, name))
}

fn text(node: roxmltree::Node) -> String {
    node.descendants()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect::<String>()
        .trim()
        .to_string()
}

/// The path of `href` resolved against `base` (the request's origin).
fn path_of(href: &str, base: &Url) -> Option<String> {
    let u = base.join(href.trim()).ok()?;
    if u.origin() != base.origin() {
        return None;
    }
    Some(u[url::Position::BeforePath..url::Position::AfterQuery].to_string())
}

/// Read a multistatus body. `None`: not XML, a DTD, or over the caps.
pub fn multistatus(body: &[u8], base: &Url) -> Option<Multistatus> {
    if body.len() > super::MAX_BODY {
        return None;
    }
    let text_body = std::str::from_utf8(body).ok()?;
    if too_deep(text_body.as_bytes()) {
        return None;
    }
    let doc = roxmltree::Document::parse_with_options(
        text_body,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: MAX_NODES,
            ..Default::default()
        },
    )
    .ok()?;
    let root = doc.root_element();
    if !is(root, DAV, "multistatus") {
        return None;
    }
    let mut out = Multistatus {
        sync_token: child(root, DAV, "sync-token").map(text),
        ..Multistatus::default()
    };
    let request_path = &base[url::Position::BeforePath..url::Position::AfterQuery];
    for r in root.children().filter(|c| is(*c, DAV, "response")) {
        if out.items.len() >= MAX_RESPONSES {
            return None;
        }
        let Some(href) = child(r, DAV, "href")
            .map(text)
            .and_then(|h| path_of(&h, base))
        else {
            continue;
        };
        let status = child(r, DAV, "status").map(|s| status_code(&text(s)));
        if status == Some(507) && href.trim_end_matches('/') == request_path.trim_end_matches('/') {
            out.truncated = true;
            continue;
        }
        let mut item = Item {
            href,
            status: status.unwrap_or(200),
            ..Item::default()
        };
        for ps in r.children().filter(|c| is(*c, DAV, "propstat")) {
            let ok = child(ps, DAV, "status").is_none_or(|s| status_code(&text(s)) / 100 == 2);
            let Some(prop) = child(ps, DAV, "prop").filter(|_| ok) else {
                continue;
            };
            read_props(prop, base, &mut item);
        }
        out.items.push(item);
    }
    Some(out)
}

/// Element nesting deeper than any multistatus needs (the parser
/// recurses, so hostile nesting is refused before parsing).
const MAX_DEPTH: usize = 64;

fn too_deep(b: &[u8]) -> bool {
    let mut depth = 0usize;
    let mut prev = 0u8;
    for w in b.windows(2) {
        let (c, next) = match w {
            [c, n] => (*c, *n),
            _ => continue,
        };
        if c == b'<' {
            if next == b'/' {
                depth = depth.saturating_sub(1);
            } else if next != b'?' && next != b'!' {
                depth = depth.saturating_add(1);
                if depth > MAX_DEPTH {
                    return true;
                }
            }
        } else if c == b'>' && prev == b'/' {
            depth = depth.saturating_sub(1);
        }
        prev = c;
    }
    false
}

fn href_in(node: roxmltree::Node, base: &Url) -> Option<String> {
    child(node, DAV, "href")
        .map(text)
        .and_then(|h| path_of(&h, base))
}

fn read_props(prop: roxmltree::Node, base: &Url, item: &mut Item) {
    for p in prop.children().filter(|c| c.is_element()) {
        let ns = p.tag_name().namespace().unwrap_or("");
        match (ns, p.tag_name().name()) {
            (DAV, "getetag") => item.etag = Some(text(p)).filter(|t| !t.is_empty()),
            (DAV, "displayname") => item.displayname = Some(text(p)),
            (DAV, "sync-token") => item.sync_token = Some(text(p)).filter(|t| !t.is_empty()),
            (CS, "getctag") => item.ctag = Some(text(p)).filter(|t| !t.is_empty()),
            (DAV, "resourcetype") => {
                item.collection = child(p, DAV, "collection").is_some();
                item.addressbook = child(p, CARDDAV, "addressbook").is_some();
                item.calendar = child(p, CALDAV, "calendar").is_some();
            }
            (DAV, "supported-report-set") => {
                item.sync_collection = p.descendants().any(|d| is(d, DAV, "sync-collection"));
            }
            (CARDDAV, "supported-address-data") => {
                item.vcard4 = p
                    .descendants()
                    .filter(|d| is(*d, CARDDAV, "address-data-type"))
                    .any(|d| d.attribute("version") == Some("4.0"));
            }
            (CALDAV, "supported-calendar-component-set") => {
                item.components = p
                    .children()
                    .filter(|c| is(*c, CALDAV, "comp"))
                    .filter_map(|c| c.attribute("name").map(str::to_ascii_uppercase))
                    .collect();
            }
            (DAV, "current-user-principal") => item.principal = href_in(p, base),
            (CARDDAV, "addressbook-home-set") => item.addressbook_home = href_in(p, base),
            (CALDAV, "calendar-home-set") => item.calendar_home = href_in(p, base),
            (CARDDAV, "address-data") | (CALDAV, "calendar-data") => {
                item.data = Some(p.text().unwrap_or("").to_string());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://dav.example.com/ab/").unwrap()
    }

    #[test]
    fn reads_prefixes_default_namespaces_and_hrefs() {
        let body = r#"<?xml version="1.0"?>
<multistatus xmlns="DAV:" xmlns:C="urn:ietf:params:xml:ns:carddav" xmlns:x="http://calendarserver.org/ns/">
 <response><href>/ab/</href><propstat><prop>
   <resourcetype><collection/><C:addressbook/></resourcetype><displayname>Work</displayname>
   <x:getctag>c1</x:getctag>
   <supported-report-set><supported-report><report><sync-collection/></report></supported-report></supported-report-set>
   <C:supported-address-data><C:address-data-type content-type="text/vcard" version="4.0"/></C:supported-address-data>
 </prop><status>HTTP/1.1 200 OK</status></propstat>
 <propstat><prop><displayname/></prop><status>HTTP/1.1 404 Not Found</status></propstat></response>
 <response><href>https://dav.example.com/ab/a%20b.vcf</href><propstat><prop><getetag>"e1"</getetag></prop></propstat></response>
 <response><href>https://evil.example.net/x.vcf</href><status>HTTP/1.1 200 OK</status></response>
 <response><href>/ab/gone.vcf</href><status>HTTP/1.1 404 Not Found</status></response>
 <sync-token>tok-2</sync-token>
</multistatus>"#;
        let m = multistatus(body.as_bytes(), &base()).unwrap();
        assert_eq!(m.sync_token.as_deref(), Some("tok-2"));
        assert_eq!(m.items.len(), 3, "the foreign href is dropped");
        let book = &m.items[0];
        assert!(book.addressbook && book.collection && book.sync_collection && book.vcard4);
        assert_eq!(book.displayname.as_deref(), Some("Work"));
        assert_eq!(book.ctag.as_deref(), Some("c1"));
        assert_eq!(m.items[1].href, "/ab/a%20b.vcf");
        assert_eq!(m.items[1].etag.as_deref(), Some("\"e1\""));
        assert_eq!(m.items[2].status, 404);
    }

    #[test]
    fn refuses_dtd_garbage_and_marks_truncation() {
        let bomb = r#"<?xml version="1.0"?><!DOCTYPE l [<!ENTITY a "aaaa"><!ENTITY b "&a;&a;&a;">]><d:multistatus xmlns:d="DAV:">&b;</d:multistatus>"#;
        assert!(multistatus(bomb.as_bytes(), &base()).is_none());
        assert!(multistatus(b"<html>no</html>", &base()).is_none());
        assert!(multistatus(b"\xff\xfe", &base()).is_none());
        let t = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/ab/</d:href><d:status>HTTP/1.1 507 Insufficient Storage</d:status></d:response><d:sync-token>t</d:sync-token></d:multistatus>"#;
        let m = multistatus(t.as_bytes(), &base()).unwrap();
        assert!(m.truncated);
        assert!(m.items.is_empty());
        let deep = format!(
            "<d:multistatus xmlns:d=\"DAV:\">{}</d:multistatus>",
            "<a>".repeat(300_000)
        );
        assert!(multistatus(deep.as_bytes(), &base()).is_none());
        assert_eq!(escape("a<&\"'>"), "a&lt;&amp;&quot;&apos;&gt;");
        assert!(multiget(true, &["/ab/1&2.vcf".into()]).contains("/ab/1&amp;2.vcf"));
    }
}
