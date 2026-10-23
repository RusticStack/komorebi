//! A fake CardDAV server for the tests: HTTPS on loopback (`fake_tls`),
//! Basic auth, a well-known redirect, the principal and home set, two
//! books (`/dav/books/a/` with sync-collection, `/dav/books/b/` with only
//! a ctag and ETags), multiget, conditional PUT and DELETE, an expired
//! token mode, a 507 truncation mode, and a redirect to another origin.
//! Every request is logged with whether it carried credentials.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use crate::mail::fake_tls::{CERT, KEY, acceptor};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

pub(crate) const USER: &str = "ana";
pub(crate) const PASS: &str = "secret";

#[derive(Default)]
pub(crate) struct Book {
    /// href -> (etag, card)
    pub(crate) cards: BTreeMap<String, (String, String)>,
    /// (seq, href) of each change, for sync tokens.
    log: Vec<(u64, String)>,
    pub(crate) ctag: u64,
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) books: BTreeMap<String, Book>,
    seq: u64,
    /// (method, path, had credentials)
    pub(crate) log: Vec<(String, String, bool)>,
    /// The next sync-collection with a token answers 403 valid-sync-token.
    pub(crate) expire_token: bool,
    /// Sync-collection answers one change per round, with a 507.
    pub(crate) truncate: bool,
    /// Writes are forbidden (403), as on a read-only calendar.
    pub(crate) read_only: bool,
}

impl State {
    pub(crate) fn put(&mut self, book: &str, name: &str, card: &str) -> String {
        self.seq += 1;
        let seq = self.seq;
        let href = format!("{book}{name}");
        let b = self.books.entry(book.to_string()).or_default();
        let etag = format!("\"e{seq}\"");
        b.cards
            .insert(href.clone(), (etag.clone(), card.to_string()));
        b.log.push((seq, href));
        b.ctag = seq;
        etag
    }

    pub(crate) fn remove(&mut self, book: &str, href: &str) {
        self.seq += 1;
        let seq = self.seq;
        let b = self.books.get_mut(book).unwrap();
        b.cards.remove(href);
        b.log.push((seq, href.to_string()));
        b.ctag = seq;
    }
}

pub(crate) struct FakeDav {
    pub(crate) port: u16,
    pub(crate) state: Arc<Mutex<State>>,
}

impl FakeDav {
    pub(crate) fn base(&self) -> String {
        format!("https://127.0.0.1:{}/", self.port)
    }

    pub(crate) fn writes(&self) -> usize {
        self.state
            .lock()
            .unwrap()
            .log
            .iter()
            .filter(|(m, _, _)| m == "PUT" || m == "DELETE")
            .count()
    }
}

pub(crate) fn card(uid: &str, name: &str, email: &str) -> String {
    format!(
        "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:{uid}\r\nFN:{name}\r\nEMAIL;TYPE=INTERNET:{email}\r\nX-KEEP:yes\r\nEND:VCARD\r\n"
    )
}

pub(crate) fn start() -> FakeDav {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut st = State::default();
    st.books.insert("/dav/books/a/".into(), Book::default());
    st.books.insert("/dav/books/b/".into(), Book::default());
    // Calendars: events with sync-collection, events with only a ctag,
    // and a task list (hidden).
    st.books.insert("/dav/cals/w/".into(), Book::default());
    st.books.insert("/dav/cals/c/".into(), Book::default());
    st.books.insert("/dav/cals/t/".into(), Book::default());
    let state = Arc::new(Mutex::new(st));
    let shared = state.clone();
    let acceptor = acceptor(CERT, KEY);
    std::thread::spawn(move || {
        for tcp in listener.incoming().flatten() {
            let (state, acceptor) = (shared.clone(), acceptor.clone());
            std::thread::spawn(move || {
                let Ok(tls) = acceptor.accept(tcp) else {
                    return;
                };
                let mut r = BufReader::new(tls);
                let mut head = String::new();
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    head.push_str(&line);
                    if line == "\r\n" {
                        break;
                    }
                }
                let header = |name: &str| {
                    head.lines().find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case(name).then(|| v.trim().to_string())
                    })
                };
                let len: usize = header("content-length")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let mut body = vec![0; len];
                r.read_exact(&mut body).ok();
                let body = String::from_utf8_lossy(&body).into_owned();
                let mut parts = head.split(' ');
                let method = parts.next().unwrap_or("").to_string();
                let path = parts.next().unwrap_or("").to_string();
                let auth = header("authorization");
                let reply = {
                    let mut st = state.lock().unwrap();
                    st.log.push((method.clone(), path.clone(), auth.is_some()));
                    respond(
                        &mut st,
                        &method,
                        &path,
                        &body,
                        auth.as_deref(),
                        &header,
                        port,
                    )
                };
                let tls = r.get_mut();
                tls.write_all(&reply).ok();
                tls.flush().ok();
                tls.shutdown().ok();
            });
        }
    });
    FakeDav { port, state }
}

fn reply(status: &str, headers: &[(&str, String)], body: &str) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    out.push_str(body);
    out.into_bytes()
}

fn ms(inner: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav" xmlns:cal="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/">{inner}</d:multistatus>"#
    )
}

fn resp(href: &str, props: &str) -> String {
    format!(
        "<d:response><d:href>{href}</d:href><d:propstat><d:prop>{props}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"
    )
}

fn respond(
    st: &mut State,
    method: &str,
    path: &str,
    body: &str,
    auth: Option<&str>,
    header: &dyn Fn(&str) -> Option<String>,
    port: u16,
) -> Vec<u8> {
    if path == "/.well-known/carddav" || path == "/.well-known/caldav" {
        return reply("301 Moved", &[("Location", "/dav/".into())], "");
    }
    if path == "/foreign" {
        // Another origin (a host name, not the IP the client used).
        return reply(
            "307 Moved",
            &[("Location", format!("https://localhost:{port}/dav/"))],
            "",
        );
    }
    let good = format!(
        "Basic {}",
        super::base64(format!("{USER}:{PASS}").as_bytes())
    );
    if auth != Some(good.as_str()) {
        return reply(
            "401 Unauthorized",
            &[("WWW-Authenticate", "Basic realm=\"x\"".into())],
            "",
        );
    }
    let ok = |b: String| {
        reply(
            "207 Multi-Status",
            &[("Content-Type", "application/xml".into())],
            &b,
        )
    };
    match (method, path) {
        ("PROPFIND", "/dav/") => ok(ms(&resp(
            "/dav/",
            "<d:current-user-principal><d:href>/dav/principal/</d:href></d:current-user-principal><d:resourcetype><d:collection/></d:resourcetype>",
        ))),
        ("PROPFIND", "/dav/principal/") => ok(ms(&resp(
            "/dav/principal/",
            "<c:addressbook-home-set><d:href>/dav/books/</d:href></c:addressbook-home-set><cal:calendar-home-set><d:href>/dav/cals/</d:href></cal:calendar-home-set>",
        ))),
        ("PROPFIND", "/dav/books/") => {
            let a = resp(
                "/dav/books/a/",
                &format!(
                    "<d:resourcetype><d:collection/><c:addressbook/></d:resourcetype><d:displayname>Work</d:displayname><d:supported-report-set><d:supported-report><d:report><d:sync-collection/></d:report></d:supported-report></d:supported-report-set><cs:getctag>{}</cs:getctag>",
                    st.books["/dav/books/a/"].ctag
                ),
            );
            let b = resp(
                "/dav/books/b/",
                "<d:resourcetype><d:collection/><c:addressbook/></d:resourcetype><d:displayname>Family</d:displayname>",
            );
            ok(ms(&format!(
                "{}{a}{b}",
                resp(
                    "/dav/books/",
                    "<d:resourcetype><d:collection/></d:resourcetype>"
                )
            )))
        }
        ("PROPFIND", "/dav/cals/") => {
            let sync = "<d:supported-report-set><d:supported-report><d:report><d:sync-collection/></d:report></d:supported-report></d:supported-report-set>";
            let w = resp(
                "/dav/cals/w/",
                &format!(
                    "<d:resourcetype><d:collection/><cal:calendar/></d:resourcetype><d:displayname>Work</d:displayname>{sync}<cal:supported-calendar-component-set><cal:comp name=\"VEVENT\"/></cal:supported-calendar-component-set>"
                ),
            );
            // No component set (as Purelymail): events.
            let c = resp(
                "/dav/cals/c/",
                "<d:resourcetype><d:collection/><cal:calendar/></d:resourcetype><d:displayname>Home</d:displayname>",
            );
            let t = resp(
                "/dav/cals/t/",
                "<d:resourcetype><d:collection/><cal:calendar/></d:resourcetype><d:displayname>Tasks</d:displayname><cal:supported-calendar-component-set><cal:comp name=\"VTODO\"/></cal:supported-calendar-component-set>",
            );
            ok(ms(&format!(
                "{}{w}{c}{t}",
                resp(
                    "/dav/cals/",
                    "<d:resourcetype><d:collection/></d:resourcetype>"
                )
            )))
        }
        ("PROPFIND", p) if st.books.contains_key(p) => {
            let b = &st.books[p];
            let kind = if p.starts_with("/dav/cals/") {
                "<cal:calendar/>"
            } else {
                "<c:addressbook/>"
            };
            let mut inner = resp(
                p,
                &format!(
                    "<d:resourcetype><d:collection/>{kind}</d:resourcetype><cs:getctag>{}</cs:getctag>",
                    b.ctag
                ),
            );
            if header("depth").as_deref() == Some("1") {
                for (h, (e, _)) in &b.cards {
                    inner.push_str(&resp(h, &format!("<d:getetag>{e}</d:getetag>")));
                }
            }
            ok(ms(&inner))
        }
        ("REPORT", p) if st.books.contains_key(p) && body.contains("sync-collection") => {
            let token = body
                .split("<d:sync-token>")
                .nth(1)
                .and_then(|t| t.split('<').next())
                .unwrap_or("");
            if !token.is_empty() && st.expire_token {
                st.expire_token = false;
                return reply(
                    "403 Forbidden",
                    &[],
                    r#"<d:error xmlns:d="DAV:"><d:valid-sync-token/></d:error>"#,
                );
            }
            let since: u64 = token.trim_start_matches("t").parse().unwrap_or(0);
            let b = &st.books[p];
            let mut changed: Vec<(u64, String)> = Vec::new();
            for (seq, h) in b.log.iter().filter(|(s, _)| *s > since) {
                changed.retain(|(_, x)| x != h);
                changed.push((*seq, h.clone()));
            }
            let truncated = st.truncate && changed.len() > 1;
            if truncated {
                changed.truncate(1);
            }
            let upto = changed
                .last()
                .map_or(b.ctag.max(since), |c| if truncated { c.0 } else { b.ctag });
            let mut inner = String::new();
            for (_, h) in &changed {
                match b.cards.get(h) {
                    Some((e, _)) => inner.push_str(&resp(h, &format!("<d:getetag>{e}</d:getetag>"))),
                    None => inner.push_str(&format!(
                        "<d:response><d:href>{h}</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>"
                    )),
                }
            }
            if truncated {
                inner.push_str(&format!(
                    "<d:response><d:href>{p}</d:href><d:status>HTTP/1.1 507 Insufficient Storage</d:status></d:response>"
                ));
            }
            inner.push_str(&format!("<d:sync-token>t{upto}</d:sync-token>"));
            ok(ms(&inner))
        }
        ("REPORT", p) if st.books.contains_key(p) => {
            let b = &st.books[p];
            let data = if body.contains("calendar-multiget") {
                "cal:calendar-data"
            } else {
                "c:address-data"
            };
            let mut inner = String::new();
            for h in body
                .split("<d:href>")
                .skip(1)
                .filter_map(|x| x.split("</d:href>").next())
            {
                let h = h.replace("&amp;", "&");
                if let Some((e, c)) = b.cards.get(&h) {
                    inner.push_str(&resp(
                        &h,
                        &format!(
                            "<d:getetag>{e}</d:getetag><{data}>{}</{data}>",
                            super::xml::escape(c)
                        ),
                    ));
                }
            }
            ok(ms(&inner))
        }
        ("PUT" | "DELETE", _) if st.read_only => reply("403 Forbidden", &[], ""),
        ("PUT", p) => {
            let book = format!("{}/", p.rsplit_once('/').map_or("", |x| x.0));
            let current = st
                .books
                .get(&book)
                .and_then(|b| b.cards.get(p))
                .map(|c| c.0.clone());
            match (header("if-none-match"), header("if-match"), current) {
                (Some(_), _, Some(_)) => reply("412 Precondition Failed", &[], ""),
                (_, Some(m), Some(c)) if m != c => reply("412 Precondition Failed", &[], ""),
                (_, Some(_), None) => reply("412 Precondition Failed", &[], ""),
                _ => {
                    let name = p.rsplit('/').next().unwrap_or("").to_string();
                    let etag = st.put(&book, &name, body);
                    reply("201 Created", &[("ETag", etag)], "")
                }
            }
        }
        ("DELETE", p) => {
            let book = format!("{}/", p.rsplit_once('/').map_or("", |x| x.0));
            let current = st
                .books
                .get(&book)
                .and_then(|b| b.cards.get(p))
                .map(|c| c.0.clone());
            match (header("if-match"), current) {
                (_, None) => reply("404 Not Found", &[], ""),
                (Some(m), Some(c)) if m != c => reply("412 Precondition Failed", &[], ""),
                _ => {
                    st.remove(&book, p);
                    reply("204 No Content", &[], "")
                }
            }
        }
        _ => reply("404 Not Found", &[], ""),
    }
}
