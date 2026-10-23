//! WebDAV for CardDAV (contacts) and CalDAV (calendars): a small HTTPS
//! client over the `fetch.rs` plumbing, the multistatus reader (`xml`),
//! discovery (`discover`, RFC 6764) and the CardDAV sync (`carddav`).
//!
//! API, shared by both services:
//! - [`Client::new`] (`base` URL, user, password, the saved certificate
//!   `pin`, `allow_private`): HTTPS only, refused under
//!   `KOMOREBI_OFFLINE`. Credentials (HTTP Basic) go only over TLS and
//!   only to `base`'s origin: a redirect to another origin fails with
//!   [`DavError::AuthRedirect`] and nothing is sent there. A host on a
//!   private network is allowed only with `allow_private` (the user typed
//!   it); a pin is honoured as IMAP does (`mail::pin::connector`, then
//!   `check_peer`).
//! - [`Client::anonymous`]: the same without credentials (discovery's
//!   `/.well-known/` step), following redirects to any public HTTPS host.
//! - [`Client::propfind`] (url, depth, body) and [`Client::report`]
//!   (url, depth, body) return the parsed [`xml::Multistatus`];
//!   [`Client::put`] (url, body, content type, [`Precondition`]) returns the
//!   new ETag if the server sent one; [`Client::delete`] (url, if-match);
//!   [`Client::get`].
//! - Request bodies are the `const` strings in `xml` (`PROPFIND_*`,
//!   `REPORT_*`) or built with [`xml::escape`]d hrefs; nothing is parsed by
//!   hand.
//! - `discover::well_known(service, domain)` finds a start URL without
//!   credentials; `discover::homes(client, url, service)` and
//!   `discover::collections(client, home)` are the authenticated steps,
//!   run only on a user's press.
//! - `sync::changes(client, collection, token, ctag, ...)` is the pull
//!   step both services share: sync-collection (RFC 6578) with tokens,
//!   507 truncation and the `valid-sync-token` reset, or else a ctag check
//!   and a Depth 1 ETag listing; `sync::multiget` fetches the changed
//!   resources 50 at a time.
//!
//! Limits: 10 MB per body, 30 s per request with its redirects, three
//! redirects.

use crate::fetch::{self, FetchError};
use native_tls::TlsConnector;
use std::fmt;
use std::io::{BufReader, Write};
use std::time::{Duration, Instant};
use url::Url;

pub mod caldav;
pub mod carddav;
pub mod discover;
#[cfg(test)]
pub(crate) mod fake;
pub mod sync;
pub mod xml;

/// The largest response body read.
pub const MAX_BODY: usize = 10 * 1024 * 1024;
const TOTAL: Duration = Duration::from_secs(30);
const MAX_REDIRECTS: usize = 3;
const USER_AGENT: &str = "Komorebi";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DavError {
    /// `KOMOREBI_OFFLINE` is set.
    Offline,
    /// Not an https URL, or a host the settings don't allow.
    Refused,
    /// The server refused the user name or password (401).
    Auth,
    /// A redirect to another origin, which credentials never follow.
    AuthRedirect,
    /// 404 or 410.
    NotFound,
    /// The server's certificate is not the saved one, or TLS failed.
    Tls(String),
    /// A body over [`MAX_BODY`], or a collection over its caps.
    TooLarge,
    /// 412: the resource changed on the server.
    Precondition,
    /// Malformed XML or a response this client can't read.
    Protocol,
    /// Another status.
    Status(u16),
    TimedOut,
    Network(String),
}

impl fmt::Display for DavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DavError::Offline => f.write_str("offline"),
            DavError::Refused => f.write_str("refused URL"),
            DavError::Auth => f.write_str("sign-in refused"),
            DavError::AuthRedirect => f.write_str("redirect to another server"),
            DavError::NotFound => f.write_str("not found"),
            DavError::Tls(e) => write!(f, "TLS: {e}"),
            DavError::TooLarge => f.write_str("too large"),
            DavError::Precondition => f.write_str("changed on the server"),
            DavError::Protocol => f.write_str("unreadable response"),
            DavError::Status(s) => write!(f, "HTTP status {s}"),
            DavError::TimedOut => f.write_str("timed out"),
            DavError::Network(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for DavError {}

impl From<FetchError> for DavError {
    fn from(e: FetchError) -> Self {
        match e {
            FetchError::Offline => DavError::Offline,
            FetchError::Refused | FetchError::NotPublic => DavError::Refused,
            FetchError::TooManyRedirects | FetchError::BadResponse => DavError::Protocol,
            FetchError::TooLarge => DavError::TooLarge,
            FetchError::Status(s) => DavError::Status(s),
            FetchError::TimedOut => DavError::TimedOut,
            FetchError::Network(e) => DavError::Network(e),
        }
    }
}

impl From<std::io::Error> for DavError {
    fn from(e: std::io::Error) -> Self {
        FetchError::from(e).into()
    }
}

/// A conditional write.
#[derive(Debug, Clone, Copy)]
pub enum Precondition<'a> {
    /// `If-None-Match: *`: create only.
    New,
    /// `If-Match`: only over this version.
    Match(&'a str),
}

/// One response.
#[derive(Debug, Clone, Default)]
pub struct Reply {
    pub status: u16,
    pub etag: Option<String>,
    pub body: Vec<u8>,
}

pub struct Client {
    origin: url::Origin,
    base: Url,
    auth: Option<String>,
    tls: TlsConnector,
    pin: Option<Vec<u8>>,
    allow_private: bool,
    /// Tests reach their server on 127.0.0.1; never set otherwise.
    loopback: bool,
}

fn basic(user: &str, pass: &str) -> String {
    format!("Basic {}", base64(format!("{user}:{pass}").as_bytes()))
}

/// Standard base64 with padding (the one use here needs no crate).
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(T[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn https(url: &str) -> Result<Url, DavError> {
    let u = Url::parse(url.trim()).map_err(|_| DavError::Refused)?;
    if u.scheme() != "https" || !u.username().is_empty() || u.password().is_some() {
        return Err(DavError::Refused);
    }
    Ok(u)
}

impl Client {
    /// A client for the DAV server at `base`, signing in as `user`.
    pub fn new(
        base: &str,
        user: &str,
        pass: &str,
        pin: Option<&[u8]>,
        allow_private: bool,
    ) -> Result<Self, DavError> {
        if crate::dev_var("KOMOREBI_OFFLINE").is_some() {
            return Err(DavError::Offline);
        }
        let base = https(base)?;
        let tls = crate::mail::pin::connector(pin).map_err(|e| DavError::Tls(e.to_string()))?;
        Ok(Client {
            origin: base.origin(),
            base,
            auth: Some(basic(user, pass)),
            tls,
            // The saved blob (its trust tag included), as `check_peer` reads it.
            pin: pin
                .filter(|b| crate::mail::pin::usable(Some(b)).is_some())
                .map(<[u8]>::to_vec),
            allow_private,
            loopback: false,
        })
    }

    /// A client that sends no credentials (discovery).
    pub fn anonymous(base: &str) -> Result<Self, DavError> {
        if crate::dev_var("KOMOREBI_OFFLINE").is_some() {
            return Err(DavError::Offline);
        }
        let base = https(base)?;
        let tls = TlsConnector::new().map_err(|e| DavError::Tls(e.to_string()))?;
        Ok(Client {
            origin: base.origin(),
            base,
            auth: None,
            tls,
            pin: None,
            allow_private: false,
            loopback: false,
        })
    }

    /// Tests only: a client for a loopback server whose certificate is
    /// `root_pem`.
    #[cfg(test)]
    pub(crate) fn for_test(base: &str, user: Option<(&str, &str)>, root_pem: &[u8]) -> Self {
        let root = native_tls::Certificate::from_pem(root_pem).unwrap();
        let tls = TlsConnector::builder()
            .add_root_certificate(root)
            .build()
            .unwrap();
        let base = Url::parse(base).unwrap();
        Client {
            origin: base.origin(),
            base,
            auth: user.map(|(u, p)| basic(u, p)),
            tls,
            pin: None,
            allow_private: false,
            loopback: true,
        }
    }

    /// The URL requests start from.
    pub fn base(&self) -> &Url {
        &self.base
    }

    /// `href` (absolute or a path) resolved against the base, refused when
    /// it leaves the base's origin while credentials are set.
    pub fn resolve(&self, href: &str) -> Result<Url, DavError> {
        let u = self.base.join(href).map_err(|_| DavError::Refused)?;
        if self.auth.is_some() && u.origin() != self.origin {
            return Err(DavError::AuthRedirect);
        }
        https(u.as_str())
    }

    fn allowed(&self, ip: std::net::IpAddr) -> bool {
        fetch::is_public(ip) || self.allow_private || (self.loopback && ip.is_loopback())
    }

    /// One request with its redirects (the same method and body; a 303
    /// becomes a GET). Any status comes back; the callers decide.
    pub fn request(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<Reply, DavError> {
        let end = Instant::now() + TOTAL;
        let mut url = self.resolve(url)?;
        let mut method = method.to_string();
        let mut body = body.to_vec();
        for _ in 0..=MAX_REDIRECTS {
            let (reply, location) = self.once(&method, &url, headers, &body, end)?;
            let Some(location) =
                location.filter(|_| matches!(reply.status, 301 | 302 | 303 | 307 | 308))
            else {
                return Ok(reply);
            };
            let next = url.join(&location).map_err(|_| DavError::Protocol)?;
            let next = https(next.as_str())?;
            if self.auth.is_some() && next.origin() != self.origin {
                return Err(DavError::AuthRedirect);
            }
            if reply.status == 303 {
                method = "GET".into();
                body.clear();
            }
            url = next;
        }
        Err(DavError::Protocol)
    }

    fn once(
        &self,
        method: &str,
        url: &Url,
        headers: &[(&str, &str)],
        body: &[u8],
        end: Instant,
    ) -> Result<(Reply, Option<String>), DavError> {
        let host = url.host_str().ok_or(DavError::Refused)?;
        let port = url.port_or_known_default().ok_or(DavError::Refused)?;
        let bare = host.trim_start_matches('[').trim_end_matches(']');
        let addrs = fetch::resolve(bare, port, end)?;
        if addrs.is_empty() || !addrs.iter().all(|a| self.allowed(a.ip())) {
            return Err(DavError::Refused);
        }
        let tcp = fetch::connect(&addrs, end)?;
        let left = fetch::time_left(end)?;
        tcp.set_read_timeout(Some(left))?;
        tcp.set_write_timeout(Some(left))?;
        let tls = self.tls.connect(bare, tcp).map_err(|e| match e {
            native_tls::HandshakeError::Failure(e) => DavError::Tls(e.to_string()),
            native_tls::HandshakeError::WouldBlock(_) => DavError::TimedOut,
        })?;
        crate::mail::pin::check_peer(&tls, self.pin.as_deref(), bare)
            .map_err(|e| DavError::Tls(e.to_string()))?;
        let mut stream = fetch::Timed { tls, end };
        let path = &url[url::Position::BeforePath..url::Position::AfterQuery];
        let host_header = match url.port() {
            Some(p) => format!("{host}:{p}"),
            None => host.to_string(),
        };
        let mut req = format!(
            "{method} {path} HTTP/1.1\r\nHost: {host_header}\r\nUser-Agent: {USER_AGENT}\r\n\
             Accept-Encoding: identity\r\nConnection: close\r\nContent-Length: {}\r\n",
            body.len()
        );
        // Credentials only to the base's origin.
        if let Some(auth) = &self.auth
            && url.origin() == self.origin
        {
            req.push_str(&format!("Authorization: {auth}\r\n"));
        }
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        stream.write_all(req.as_bytes())?;
        stream.write_all(body)?;
        stream.flush()?;
        let mut r = BufReader::new(stream);
        let head = fetch::read_head(&mut r, MAX_BODY)?;
        let body = if method == "HEAD" || head.status == 204 || head.status == 304 {
            Vec::new()
        } else {
            fetch::read_body(&mut r, &head, MAX_BODY, None)?
        };
        Ok((
            Reply {
                status: head.status,
                etag: head.etag,
                body,
            },
            head.location,
        ))
    }

    fn check(reply: Reply) -> Result<Reply, DavError> {
        match reply.status {
            200..=299 => Ok(reply),
            401 => Err(DavError::Auth),
            404 | 410 => Err(DavError::NotFound),
            412 => Err(DavError::Precondition),
            s => Err(DavError::Status(s)),
        }
    }

    /// PROPFIND `url` at `depth` ("0" or "1").
    pub fn propfind(
        &self,
        url: &str,
        depth: &str,
        body: &str,
    ) -> Result<xml::Multistatus, DavError> {
        let reply = self.request(
            "PROPFIND",
            url,
            &[
                ("Depth", depth),
                ("Content-Type", "application/xml; charset=utf-8"),
            ],
            body.as_bytes(),
        )?;
        let reply = Self::check(reply)?;
        xml::multistatus(&reply.body, &self.resolve(url)?).ok_or(DavError::Protocol)
    }

    /// REPORT `url`. A 403 or 409 comes back as `Status` (the sync code
    /// reads `valid-sync-token` from it).
    pub fn report(&self, url: &str, depth: &str, body: &str) -> Result<xml::Multistatus, DavError> {
        let reply = self.request(
            "REPORT",
            url,
            &[
                ("Depth", depth),
                ("Content-Type", "application/xml; charset=utf-8"),
            ],
            body.as_bytes(),
        )?;
        if matches!(reply.status, 403 | 409)
            && String::from_utf8_lossy(&reply.body).contains("valid-sync-token")
        {
            return Err(DavError::Status(reply.status));
        }
        let reply = Self::check(reply)?;
        xml::multistatus(&reply.body, &self.resolve(url)?).ok_or(DavError::Protocol)
    }

    pub fn get(&self, url: &str) -> Result<Reply, DavError> {
        Self::check(self.request("GET", url, &[], &[])?)
    }

    /// PUT `body` at `url` under `pre`; the new ETag, if the server sent it.
    pub fn put(
        &self,
        url: &str,
        body: &[u8],
        content_type: &str,
        pre: Precondition,
    ) -> Result<Option<String>, DavError> {
        let cond = match pre {
            Precondition::New => ("If-None-Match", "*"),
            Precondition::Match(e) => ("If-Match", e),
        };
        let reply = self.request("PUT", url, &[("Content-Type", content_type), cond], body)?;
        Ok(Self::check(reply)?.etag)
    }

    /// DELETE `url` if it is still at `etag` (none: unconditionally).
    pub fn delete(&self, url: &str, etag: Option<&str>) -> Result<(), DavError> {
        self.delete_with(url, etag, &[])
    }

    /// [`Client::delete`] with `extra` headers (`Schedule-Reply: F`).
    pub fn delete_with(
        &self,
        url: &str,
        etag: Option<&str>,
        extra: &[(&str, &str)],
    ) -> Result<(), DavError> {
        let mut headers: Vec<(&str, &str)> = etag.map(|e| ("If-Match", e)).into_iter().collect();
        headers.extend_from_slice(extra);
        let reply = self.request("DELETE", url, &headers, &[])?;
        match reply.status {
            404 | 410 => Ok(()),
            _ => Self::check(reply).map(drop),
        }
    }
}
