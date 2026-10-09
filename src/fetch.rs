//! HTTPS GET for remote images the user chose to load, and nothing else.
//!
//! A plain HTTP/1.1 client over `std::net` and `native-tls`, HTTPS only:
//! an `http://` or `//host` image URL from the message is requested as
//! `https://` (a host without HTTPS just fails; plain HTTP is never
//! spoken), and a redirect to `http://` is refused. No cookies, no Referer, no
//! credentials, a fixed User-Agent and no compression. Hosts that resolve
//! to loopback, private, link-local or other non-public addresses are
//! refused, and the vetted addresses are the ones connected to. Time,
//! redirects and body size are capped. `KOMOREBI_OFFLINE` refuses every
//! request.

use native_tls::{TlsConnector, TlsStream};
use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};
use url::Url;

/// The largest body accepted.
pub const MAX_BODY: usize = 10 * 1024 * 1024;
const MAX_HEAD: u64 = 64 * 1024;
const MAX_REDIRECTS: usize = 3;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(15);
const USER_AGENT: &str = "Mozilla/5.0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// `KOMOREBI_OFFLINE` is set.
    Offline,
    /// Not an `https://` (or upgradable `http://`) URL, or it carries
    /// credentials.
    Refused,
    /// The host resolves to a loopback, private or link-local address.
    NotPublic,
    TooManyRedirects,
    /// The body is over [`MAX_BODY`].
    TooLarge,
    /// A status other than 200 or a followed redirect.
    Status(u16),
    /// A malformed or compressed response.
    BadResponse,
    TimedOut,
    Network(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Offline => f.write_str("offline"),
            FetchError::Refused => f.write_str("not an https URL"),
            FetchError::NotPublic => f.write_str("host is not on the public internet"),
            FetchError::TooManyRedirects => f.write_str("too many redirects"),
            FetchError::TooLarge => f.write_str("response too large"),
            FetchError::Status(s) => write!(f, "HTTP status {s}"),
            FetchError::BadResponse => f.write_str("malformed response"),
            FetchError::TimedOut => f.write_str("timed out"),
            FetchError::Network(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for FetchError {}

impl From<io::Error> for FetchError {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => FetchError::TimedOut,
            io::ErrorKind::InvalidData => FetchError::BadResponse,
            _ => FetchError::Network(e.to_string()),
        }
    }
}

/// GET `url` and return the body.
pub fn get(url: &str) -> Result<Vec<u8>, FetchError> {
    if crate::dev_var("KOMOREBI_OFFLINE").is_some() {
        return Err(FetchError::Offline);
    }
    let tls = TlsConnector::new().map_err(|e| FetchError::Network(e.to_string()))?;
    Client {
        tls,
        loopback_ok: false,
    }
    .get(url)
}

struct Client {
    tls: TlsConnector,
    /// Lets the tests reach their server on 127.0.0.1; never set otherwise.
    loopback_ok: bool,
}

/// A response head: the status and the headers this client reads.
struct Head {
    status: u16,
    length: Option<usize>,
    chunked: bool,
    location: Option<String>,
}

impl Client {
    fn get(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        let end = Instant::now() + TOTAL_TIMEOUT;
        let mut url = https_url(&upgraded(url))?;
        for _ in 0..=MAX_REDIRECTS {
            match self.request(&url, end)? {
                Ok(body) => return Ok(body),
                Err(location) => {
                    url = https_url(
                        url.join(&location)
                            .map_err(|_| FetchError::Refused)?
                            .as_str(),
                    )?
                }
            }
        }
        Err(FetchError::TooManyRedirects)
    }

    /// One request: the body, or where a redirect points.
    fn request(&self, url: &Url, end: Instant) -> Result<Result<Vec<u8>, String>, FetchError> {
        let host = url.host_str().ok_or(FetchError::Refused)?;
        let port = url.port_or_known_default().ok_or(FetchError::Refused)?;
        // An IPv6 literal comes bracketed; resolve and verify it bare.
        let bare = host.trim_start_matches('[').trim_end_matches(']');
        let addrs = resolve(bare, port, end)?;
        if addrs.is_empty() || !addrs.iter().all(|a| self.allowed(a.ip())) {
            return Err(FetchError::NotPublic);
        }
        let tcp = connect(&addrs, end)?;
        // The handshake runs under the same overall limit.
        let left = time_left(end)?;
        tcp.set_read_timeout(Some(left))?;
        tcp.set_write_timeout(Some(left))?;
        let tls = self.tls.connect(bare, tcp).map_err(|e| match e {
            native_tls::HandshakeError::Failure(e) => FetchError::Network(e.to_string()),
            native_tls::HandshakeError::WouldBlock(_) => FetchError::TimedOut,
        })?;
        let mut stream = Timed { tls, end };
        let path = &url[url::Position::BeforePath..url::Position::AfterQuery];
        let host_header = match url.port() {
            Some(p) => format!("{host}:{p}"),
            None => host.to_string(),
        };
        let req = format!(
            "GET {path} HTTP/1.1\r\nHost: {host_header}\r\nUser-Agent: {USER_AGENT}\r\n\
             Accept: image/*\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(req.as_bytes())?;
        stream.flush()?;
        let mut r = BufReader::new(stream);
        let head = read_head(&mut r)?;
        match head.status {
            200 => read_body(&mut r, &head).map(Ok),
            301 | 302 | 303 | 307 | 308 => head.location.map(Err).ok_or(FetchError::BadResponse),
            s => Err(FetchError::Status(s)),
        }
    }

    fn allowed(&self, ip: IpAddr) -> bool {
        is_public(ip) || self.loopback_ok && ip.is_loopback()
    }
}

/// A message's image URL with `http://` or a scheme-relative `//` made
/// `https://` (an explicit `:80` is dropped with it).
fn upgraded(url: &str) -> String {
    let t = url.trim();
    let rest = if t
        .get(..7)
        .is_some_and(|h| h.eq_ignore_ascii_case("http://"))
    {
        t.get(7..).unwrap_or("")
    } else if let Some(r) = t.strip_prefix("//") {
        r
    } else {
        return t.to_string();
    };
    let (host, path) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    let host = host.strip_suffix(":80").unwrap_or(host);
    format!("https://{host}{path}")
}

/// `url` parsed, if it is `https://` without credentials.
fn https_url(url: &str) -> Result<Url, FetchError> {
    let u = Url::parse(url.trim()).map_err(|_| FetchError::Refused)?;
    if u.scheme() != "https" || !u.username().is_empty() || u.password().is_some() {
        return Err(FetchError::Refused);
    }
    Ok(u)
}

fn time_left(end: Instant) -> io::Result<Duration> {
    let left = end.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(io::ErrorKind::TimedOut.into());
    }
    Ok(left)
}

/// The addresses of `host`. The system resolver has no timeout of its own,
/// so it runs on a thread this request stops waiting for at the deadline.
fn resolve(host: &str, port: u16, end: Instant) -> Result<Vec<SocketAddr>, FetchError> {
    let (tx, rx) = std::sync::mpsc::channel();
    let host = host.to_string();
    std::thread::Builder::new()
        .name("resolve".into())
        .spawn(move || {
            tx.send(
                (host.as_str(), port)
                    .to_socket_addrs()
                    .map(Iterator::collect),
            )
            .ok();
        })?;
    match rx.recv_timeout(time_left(end)?) {
        Ok(addrs) => Ok(addrs?),
        Err(_) => Err(FetchError::TimedOut),
    }
}

fn connect(addrs: &[SocketAddr], end: Instant) -> Result<TcpStream, FetchError> {
    let mut last = FetchError::TimedOut;
    for a in addrs {
        match TcpStream::connect_timeout(a, time_left(end)?.min(CONNECT_TIMEOUT)) {
            Ok(s) => return Ok(s),
            Err(e) => last = e.into(),
        }
    }
    Err(last)
}

/// Whether `ip` is a public unicast address.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public_v4(v4);
            }
            let s = v6.segments();
            // NAT64 (64:ff9b::/96) and the old IPv4-compatible form (::/96)
            // carry an IPv4 address in their low bits.
            if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] || s[..6] == [0; 6] {
                let [.., a, b, c, d] = v6.octets();
                return public_v4(Ipv4Addr::new(a, b, c, d));
            }
            // 6to4 (2002::/16) and Teredo (2001::/32) tunnel to IPv4 hosts.
            !(v6.is_loopback()
                || s[0] == 0x2002
                || (s[0] == 0x2001 && s[1] == 0)
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80 // link-local
                || (s[0] & 0xffc0) == 0xfec0 // site-local
                || (s[0] == 0x2001 && s[1] == 0x0db8)) // documentation
        }
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0
        || (a == 100 && (64..128).contains(&b)) // shared address space (CGNAT)
        || (a == 192 && b == 0 && c == 0) // IETF protocol assignments
        || (a == 198 && (b == 18 || b == 19)) // benchmarking
        || a >= 240)
}

fn read_line(r: &mut impl BufRead, limit: u64) -> Result<String, FetchError> {
    let mut line = Vec::new();
    r.take(limit).read_until(b'\n', &mut line)?;
    if line.last() != Some(&b'\n') {
        return Err(FetchError::BadResponse);
    }
    String::from_utf8(line).map_err(|_| FetchError::BadResponse)
}

fn read_head(r: &mut impl BufRead) -> Result<Head, FetchError> {
    let mut left = MAX_HEAD;
    let mut next = |r: &mut _| -> Result<String, FetchError> {
        let line = read_line(r, left)?;
        left = left.saturating_sub(line.len() as u64);
        Ok(line)
    };
    let status_line = next(r)?;
    let mut parts = status_line.split_ascii_whitespace();
    let status = match (parts.next(), parts.next()) {
        (Some(v), Some(code)) if v.starts_with("HTTP/1.") => {
            code.parse().map_err(|_| FetchError::BadResponse)?
        }
        _ => return Err(FetchError::BadResponse),
    };
    let mut head = Head {
        status,
        length: None,
        chunked: false,
        location: None,
    };
    loop {
        let line = next(r)?;
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            return Ok(head);
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(FetchError::BadResponse);
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => {
                let n: u64 = value.parse().map_err(|_| FetchError::BadResponse)?;
                if n > MAX_BODY as u64 {
                    return Err(FetchError::TooLarge);
                }
                head.length = Some(n as usize);
            }
            "transfer-encoding" => {
                let v = value.to_ascii_lowercase();
                if v != "chunked" && v != "identity" {
                    return Err(FetchError::BadResponse);
                }
                head.chunked = v == "chunked";
            }
            "content-encoding" if !value.eq_ignore_ascii_case("identity") => {
                return Err(FetchError::BadResponse);
            }
            "location" => head.location = Some(value.to_string()),
            _ => {}
        }
    }
}

fn read_body(r: &mut impl BufRead, head: &Head) -> Result<Vec<u8>, FetchError> {
    let mut body = Vec::new();
    if head.chunked {
        loop {
            let line = read_line(r, 1024)?;
            let size = line.trim().split(';').next().unwrap_or("").trim();
            let size = usize::from_str_radix(size, 16).map_err(|_| FetchError::BadResponse)?;
            if size > MAX_BODY - body.len() {
                return Err(FetchError::TooLarge);
            }
            if size == 0 {
                return Ok(body);
            }
            let at = body.len();
            body.resize(at + size, 0);
            r.read_exact(&mut body[at..])?;
            if read_line(r, 2)?.trim() != "" {
                return Err(FetchError::BadResponse);
            }
        }
    }
    if let Some(n) = head.length {
        body.resize(n, 0);
        r.read_exact(&mut body)?;
        return Ok(body);
    }
    r.take(MAX_BODY as u64 + 1).read_to_end(&mut body)?;
    if body.len() > MAX_BODY {
        return Err(FetchError::TooLarge);
    }
    Ok(body)
}

/// A TLS stream whose every read and write is bounded by the request's
/// overall deadline.
struct Timed {
    tls: TlsStream<TcpStream>,
    end: Instant,
}

impl Timed {
    fn stream(&mut self) -> io::Result<&mut TlsStream<TcpStream>> {
        let left = time_left(self.end)?;
        self.tls.get_ref().set_read_timeout(Some(left))?;
        self.tls.get_ref().set_write_timeout(Some(left))?;
        Ok(&mut self.tls)
    }
}

impl Read for Timed {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream()?.read(buf)
    }
}

impl Write for Timed {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream()?.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream()?.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use native_tls::{Certificate, Identity, TlsAcceptor};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex, OnceLock};

    const CERT: &[u8] = include_bytes!("../tests/fixtures/tls/test-cert.pem");
    const KEY: &[u8] = include_bytes!("../tests/fixtures/tls/test-key.pem");

    /// One acceptor per test run (Windows keeps its key in a container).
    fn acceptor() -> Arc<TlsAcceptor> {
        static ACCEPTOR: OnceLock<Arc<TlsAcceptor>> = OnceLock::new();
        ACCEPTOR
            .get_or_init(|| {
                let id = Identity::from_pkcs8(CERT, KEY).unwrap();
                Arc::new(TlsAcceptor::new(id).unwrap())
            })
            .clone()
    }

    fn client() -> Client {
        let tls = TlsConnector::builder()
            .add_root_certificate(Certificate::from_pem(CERT).unwrap())
            .build()
            .unwrap();
        Client {
            tls,
            loopback_ok: true,
        }
    }

    /// The fake server's answer to a request path.
    fn respond(path: &str, port: u16) -> Vec<u8> {
        let redirect = |to: String| {
            format!(
                "HTTP/1.1 302 Found\r\nLocation: {to}\r\nSet-Cookie: track=1; Path=/\r\n\
                 Content-Length: 0\r\n\r\n"
            )
            .into_bytes()
        };
        match path {
            "/img" => b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nimage".to_vec(),
            "/chunked" => b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                3\r\nima\r\n2;x=y\r\nge\r\n0\r\n\r\n"
                .to_vec(),
            "/eof" => b"HTTP/1.1 200 OK\r\n\r\nimage".to_vec(),
            "/big" => format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                MAX_BODY + 1
            )
            .into_bytes(),
            "/bigchunk" => format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
                MAX_BODY + 1
            )
            .into_bytes(),
            "/gzip" => {
                b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 5\r\n\r\nimage"
                    .to_vec()
            }
            "/missing" => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
            "/to-http" => redirect(format!("http://127.0.0.1:{port}/img")),
            // /hop/N redirects N + 1 times (the last one relative) to /img.
            p if p.starts_with("/hop/") => match p[5..].parse::<u32>().unwrap() {
                0 => redirect("/img".into()),
                n => redirect(format!("https://127.0.0.1:{port}/hop/{}", n - 1)),
            },
            _ => b"HTTP/1.1 500 Oops\r\nContent-Length: 0\r\n\r\n".to_vec(),
        }
    }

    /// A fake HTTPS server on 127.0.0.1; returns its port and the request
    /// heads it received.
    fn serve() -> (u16, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let acceptor = acceptor();
        std::thread::spawn(move || {
            for tcp in listener.incoming().flatten() {
                let (log, acceptor) = (log.clone(), acceptor.clone());
                std::thread::spawn(move || {
                    let Ok(mut tls) = acceptor.accept(tcp) else {
                        return;
                    };
                    let mut head = Vec::new();
                    let mut byte = [0];
                    while !head.ends_with(b"\r\n\r\n") && tls.read(&mut byte).unwrap_or(0) == 1 {
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8_lossy(&head).into_owned();
                    let path = head.split(' ').nth(1).unwrap_or("").to_string();
                    log.lock().unwrap().push(head);
                    tls.write_all(&respond(&path, port)).ok();
                    tls.flush().ok();
                    tls.shutdown().ok();
                });
            }
        });
        (port, seen)
    }

    #[test]
    fn fetches_bodies_and_follows_up_to_three_redirects() {
        let (port, seen) = serve();
        let c = client();
        let url = |p: &str| format!("https://127.0.0.1:{port}{p}");
        assert_eq!(c.get(&url("/img")).unwrap(), b"image");
        assert_eq!(c.get(&url("/chunked")).unwrap(), b"image");
        assert_eq!(c.get(&url("/eof")).unwrap(), b"image");
        assert_eq!(c.get(&url("/hop/2")).unwrap(), b"image");
        assert_eq!(c.get(&url("/hop/3")), Err(FetchError::TooManyRedirects));
        assert_eq!(c.get(&url("/missing")), Err(FetchError::Status(404)));
        assert_eq!(c.get(&url("/gzip")), Err(FetchError::BadResponse));

        // The redirects set cookies; none is ever sent back, and requests
        // carry no Referer or credentials, only the fixed User-Agent.
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 13);
        for head in seen.iter() {
            let lower = head.to_ascii_lowercase();
            assert!(!lower.contains("\r\ncookie:"), "{head}");
            assert!(!lower.contains("\r\nreferer:"), "{head}");
            assert!(!lower.contains("\r\nauthorization:"), "{head}");
            assert!(head.contains("\r\nUser-Agent: Mozilla/5.0\r\n"), "{head}");
            assert!(head.contains("\r\nAccept-Encoding: identity\r\n"), "{head}");
        }
    }

    #[test]
    fn caps_the_body_size() {
        let (port, _) = serve();
        let c = client();
        for p in ["/big", "/bigchunk"] {
            let url = format!("https://127.0.0.1:{port}{p}");
            assert_eq!(c.get(&url), Err(FetchError::TooLarge), "{p}");
        }
    }

    #[test]
    fn http_and_scheme_relative_urls_are_fetched_over_https() {
        let (port, seen) = serve();
        let c = client();
        for url in [
            format!("http://127.0.0.1:{port}/img"),
            format!("HTTP://127.0.0.1:{port}/img"),
            format!("//127.0.0.1:{port}/img"),
        ] {
            assert_eq!(c.get(&url).as_deref(), Ok(&b"image"[..]), "{url}");
        }
        assert_eq!(seen.lock().unwrap().len(), 3);
        assert_eq!(upgraded("http://a.example:80/x?y"), "https://a.example/x?y");
        assert_eq!(upgraded("http://a.example"), "https://a.example");
        assert_eq!(upgraded(" https://a.example/ "), "https://a.example/");
        assert_eq!(upgraded("data:image/png,x"), "data:image/png,x");
    }

    #[test]
    fn refuses_http_credentials_and_private_hosts() {
        let (port, seen) = serve();
        let c = client();
        let refused = [
            format!("https://user:pw@127.0.0.1:{port}/img"),
            format!("http://user:pw@127.0.0.1:{port}/img"),
            "ftp://127.0.0.1/img".into(),
            "javascript:alert(1)".into(),
        ];
        for url in &refused {
            assert_eq!(c.get(url), Err(FetchError::Refused), "{url}");
        }
        // Nothing reached the server for those.
        assert!(seen.lock().unwrap().is_empty());
        // A redirect to http:// is refused too, not upgraded.
        let to_http = format!("https://127.0.0.1:{port}/to-http");
        assert_eq!(c.get(&to_http), Err(FetchError::Refused));
        assert_eq!(seen.lock().unwrap().len(), 1);

        // Outside the tests loopback is refused like any non-public host.
        let real = Client {
            loopback_ok: false,
            ..client()
        };
        assert_eq!(
            real.get(&format!("https://127.0.0.1:{port}/img")),
            Err(FetchError::NotPublic)
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn only_public_addresses_are_allowed() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "::1",
            "::",
            "fe80::1",
            "fd00::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::a00:1",
            "::7f00:1",
            "2002:7f00:1::1",
            "2001:0:4136:e378::1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "93.184.216.34",
            "1.1.1.1",
            "2606:4700:4700::1111",
            "64:ff9b::101:101",
        ] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
    }
}
