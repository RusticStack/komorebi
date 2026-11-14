//! "Find settings": the server settings of an email domain, looked up only
//! when the user presses the button, and only ever used to prefill the
//! account form.
//!
//! Order:
//! 1. the built-in providers by the address's domain (`providers::guess`:
//!    gmail.com is Gmail);
//! 2. the domain's DNS MX records, mapped to a built-in provider by its MX
//!    host suffixes (`providers::by_mx`: a custom domain whose MX is
//!    mail.postale.io is hosted by Postale.io); when no MX host maps to one,
//!    the PTR (reverse) names of the first MX hosts' addresses, mapped the
//!    same way;
//! 3. DNS SRV records (RFC 6186 and RFC 8314: `_imaps._tcp` and
//!    `_submissions._tcp`, then `_imap._tcp` and `_submission._tcp` for
//!    STARTTLS);
//! 4. the provider's own autoconfig file over HTTPS
//!    (`https://autoconfig.<domain>/mail/config-v1.1.xml`, then
//!    `https://<domain>/.well-known/autoconfig/mail/config-v1.1.xml`).
//!
//! The DNS queries (MX, the MX hosts' addresses, PTR, SRV) go to the system
//! resolver only. Nothing is sent to Mozilla, an MX host or any other third
//! party, and the request URLs carry the domain only, never the address.
//!
//! DNS and HTTP answers are unauthenticated, and TLS cannot tell a hijacked
//! answer from a real one (an attacker's server can have a valid certificate
//! of its own), so:
//! - an MX or PTR answer can only **select a built-in provider**, whose
//!   servers are compiled in ([`crate::providers::PRESETS`]). The MX or PTR
//!   host name itself never reaches the form, the note or anything else: a
//!   hostile answer can at worst pick another well-known provider (the
//!   setup then names it: "Your mail is hosted by …", before any password
//!   is sent), never a server of its own;
//! - a server named by SRV or autoconfig on a different registrable domain
//!   than the email address is not offered for the form: it is reported and
//!   left to the user to type by hand. Plaintext (`socketType` plain) and
//!   password schemes Komorebi cannot speak are ignored.
//!
//! The lookups are closures so the tests inject their answers; the real ones
//! are [`system_lookups`] (the Windows resolver through `dnsapi.dll`, loaded
//! only when a lookup runs, the standard library's resolver for the MX
//! hosts' addresses, and [`crate::fetch`]).

use crate::fetch::{self, FetchError};
use crate::providers::{self, Security, Server};
use std::net::IpAddr;
use std::time::Duration;

/// The largest autoconfig file read.
pub const MAX_XML: usize = 64 * 1024;
/// The most XML nodes parsed.
const MAX_NODES: u32 = 2000;
/// The time one autoconfig request may take.
const FETCH_TIME: Duration = Duration::from_secs(5);

/// A server found, and its sign-in name when the provider says it is not
/// the email address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub server: Server,
    pub user: Option<String>,
}

/// The servers found; either may be missing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Discovered {
    pub imap: Option<Found>,
    pub smtp: Option<Found>,
}

/// Where a result came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The built-in provider table, by the address's domain.
    Builtin,
    /// A built-in provider, by the domain's MX (or the MX hosts' PTR) names.
    Mx,
    /// The domain's SRV records.
    Srv,
    /// The provider's autoconfig file.
    Autoconfig,
}

/// What a lookup found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub found: Discovered,
    /// Where the servers in `found` came from, in the order tried.
    pub sources: Vec<Source>,
    /// Hosts the domain's settings named that are on another domain, and so
    /// were not taken.
    pub refused: Vec<String>,
    /// The built-in provider (an index into `providers::PRESETS`) the
    /// address's domain or its MX records name. `found` then holds its
    /// servers, or nothing for one Komorebi cannot sign in to.
    pub preset: Option<usize>,
}

/// One DNS MX record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MxRecord {
    pub preference: u16,
    pub exchange: String,
}

/// The most MX hosts compared with the presets, and the most whose
/// addresses' PTR names are looked up when none matched.
const MX_HOSTS: usize = 8;
const PTR_HOSTS: usize = 2;

impl Outcome {
    pub fn is_empty(&self) -> bool {
        self.found.imap.is_none() && self.found.smtp.is_none()
    }
}

/// One DNS SRV record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SrvRecord {
    pub priority: u16,
    pub weight: u16,
    pub port: u16,
    pub target: String,
}

/// The body of an HTTPS URL for a domain (the URL, then the domain).
pub type FetchFn<'a> = dyn Fn(&str, &str) -> Result<Vec<u8>, FetchError> + 'a;

/// What a lookup uses: the MX records of a domain, the PTR names of a
/// host's addresses, the SRV records of a name (each empty: none, or the
/// lookup failed) and the body of an HTTPS URL for the domain.
pub struct Lookups<'a> {
    pub mx: &'a dyn Fn(&str) -> Vec<MxRecord>,
    pub ptr: &'a dyn Fn(&str) -> Vec<String>,
    pub srv: &'a dyn Fn(&str) -> Vec<SrvRecord>,
    pub fetch: &'a FetchFn<'a>,
}

/// The real lookups: the system resolver and HTTPS.
pub fn system_lookups() -> Lookups<'static> {
    Lookups {
        mx: &query_mx_online,
        ptr: &query_ptr_online,
        srv: &query_srv_online,
        fetch: &fetch_autoconfig,
    }
}

fn offline() -> bool {
    crate::dev_var("KOMOREBI_OFFLINE").is_some()
}

/// The SRV records of `name`, none under `KOMOREBI_OFFLINE` (the web lookup
/// refuses there too, in `fetch`).
fn query_srv_online(name: &str) -> Vec<SrvRecord> {
    if offline() {
        return Vec::new();
    }
    dns::query_srv(name)
}

/// The MX records of `domain`, none under `KOMOREBI_OFFLINE`.
fn query_mx_online(domain: &str) -> Vec<MxRecord> {
    if offline() {
        return Vec::new();
    }
    dns::query_mx(domain)
}

/// The PTR names of `host`'s addresses (at most four addresses), none under
/// `KOMOREBI_OFFLINE`. The addresses come from the system resolver too.
fn query_ptr_online(host: &str) -> Vec<String> {
    use std::net::ToSocketAddrs;
    if offline() || valid_hostname(host).is_none() {
        return Vec::new();
    }
    let Ok(addrs) = (host, 0u16).to_socket_addrs() else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for addr in addrs.take(4) {
        names.extend(dns::query_ptr(&reverse_name(addr.ip())));
    }
    names
}

/// The reverse DNS name of an address: `4.3.2.1.in-addr.arpa` for 1.2.3.4,
/// the nibbles of an IPv6 address under `ip6.arpa`.
pub fn reverse_name(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, d] = v4.octets();
            format!("{d}.{c}.{b}.{a}.in-addr.arpa")
        }
        IpAddr::V6(v6) => {
            let mut s = String::with_capacity(72);
            for byte in v6.octets().iter().rev() {
                s.push_str(&format!("{:x}.{:x}.", byte & 0xf, byte >> 4));
            }
            s.push_str("ip6.arpa");
            s
        }
    }
}

/// The usable SRV records of `name` in the order to try them (none under
/// `KOMOREBI_OFFLINE`), for DAV discovery (`crate::dav::discover`).
pub fn srv_ordered(name: &str) -> Vec<SrvRecord> {
    srv_order(&query_srv_online(name))
}

fn fetch_autoconfig(url: &str, domain: &str) -> Result<Vec<u8>, FetchError> {
    let domain = domain.to_string();
    let allowed = move |u: &url::Url| redirect_allowed(u, &domain);
    fetch::get_with(
        url,
        &fetch::Options {
            accept: "application/xml, text/xml;q=0.9",
            max_body: MAX_XML,
            total: FETCH_TIME,
            redirect_ok: &allowed,
            user_agent: fetch::USER_AGENT,
            progress: None,
        },
    )
}

/// Whether an autoconfig request may follow a redirect to `to`: HTTPS (the
/// client refuses anything else before asking) on the email's own
/// registrable domain.
pub fn redirect_allowed(to: &url::Url, domain: &str) -> bool {
    to.scheme() == "https"
        && matches!(to.host(), Some(url::Host::Domain(h)) if same_domain(h, domain))
}

/// The local part and the (ASCII, lower case) domain of a plain address, or
/// `None` for anything that is not a simple `local@domain.tld`.
pub fn parse_email(email: &str) -> Option<(String, String)> {
    let email = email.trim();
    let (local, domain) = email.rsplit_once('@')?;
    let atext = |c: char| c.is_ascii_alphanumeric() || "!#$%&'*+/=?^_`{|}~-.".contains(c);
    if local.is_empty()
        || local.len() > 64
        || !local.chars().all(atext)
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
    {
        return None;
    }
    let domain = idna::domain_to_ascii(domain.trim_end_matches('.')).ok()?;
    let domain = domain.to_ascii_lowercase();
    (valid_hostname(&domain).is_some() && domain.contains('.')).then(|| (local.to_string(), domain))
}

/// Whether `email` is an address "Find settings" can use.
pub fn looks_valid(email: &str) -> bool {
    parse_email(email).is_some()
}

/// `host` as a lower case name with no trailing dot, if it is a plain DNS
/// name (letters, digits, `-` and `_` in labels) and not an address.
fn valid_hostname(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || host.len() > 253 || host.parse::<IpAddr>().is_ok() {
        return None;
    }
    host.split('.')
        .all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && l.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
        .then_some(host)
}

/// Suffixes of two labels under which names are registered, so the
/// registrable domain has three (a short list, not the public suffix list).
const TWO_LABEL_SUFFIXES: &[&str] = &[
    "co.uk",
    "org.uk",
    "ac.uk",
    "gov.uk",
    "me.uk",
    "ltd.uk",
    "plc.uk",
    "net.uk",
    "sch.uk",
    "com.au",
    "net.au",
    "org.au",
    "edu.au",
    "gov.au",
    "co.nz",
    "org.nz",
    "net.nz",
    "co.jp",
    "ne.jp",
    "or.jp",
    "ac.jp",
    "com.br",
    "net.br",
    "org.br",
    "com.cn",
    "net.cn",
    "org.cn",
    "com.hk",
    "com.sg",
    "com.tw",
    "com.mx",
    "com.ar",
    "com.tr",
    "co.za",
    "co.in",
    "co.il",
    "co.kr",
    "or.kr",
    "com.pt",
    "org.pt",
    "gov.pt",
    "edu.pt",
    // Free subdomains anyone can register: one tenant is not another's.
    "duckdns.org",
    "dyndns.org",
    "ddns.net",
    "no-ip.org",
    "no-ip.com",
    "hopto.org",
    "zapto.org",
    "myftp.org",
    "github.io",
    "gitlab.io",
    "herokuapp.com",
    "appspot.com",
    "blogspot.com",
    "pages.dev",
    "workers.dev",
    "netlify.app",
    "vercel.app",
    "web.app",
    "firebaseapp.com",
    "onrender.com",
    "fly.dev",
    "nip.io",
    "sslip.io",
];

/// The registrable domain of a lower case name: its last two labels, three
/// under a suffix of [`TWO_LABEL_SUFFIXES`].
fn registrable(host: &str) -> String {
    let labels: Vec<&str> = host.split('.').collect();
    let n = labels.len();
    if n < 2 {
        return host.to_string();
    }
    let last_two = labels[n - 2..].join(".");
    let keep = if n >= 3 && TWO_LABEL_SUFFIXES.contains(&last_two.as_str()) {
        3
    } else {
        2
    };
    labels[n - keep..].join(".")
}

/// Whether `host` is on the same registrable domain as the email `domain`
/// (an address, an empty or odd name never is).
pub fn same_domain(host: &str, domain: &str) -> bool {
    let (Some(host), Some(domain)) = (valid_hostname(host), valid_hostname(domain)) else {
        return false;
    };
    registrable(&host) == registrable(&domain)
}

/// `template` with its `%NAME%` placeholders replaced from `vars`; `None` if
/// it has any other `%`. The values may hold `%` themselves: they are not
/// scanned again.
fn expand(template: &str, vars: &[(&str, &str)]) -> Option<String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('%')?;
        let name = &after[..end];
        let value = vars.iter().find(|(n, _)| *n == name)?.1;
        out.push_str(value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// The servers an autoconfig file (Thunderbird's `config-v1.1.xml`) names for
/// `email`: the first IMAP and the first SMTP server with implicit TLS or
/// STARTTLS and password sign-in. Strict: a DTD, more than [`MAX_XML`] bytes
/// or [`MAX_NODES`] nodes, a missing or zero port, an unknown placeholder or
/// a plaintext server is skipped, never repaired. The hosts are not checked
/// against the email's domain here (see [`discover`]).
pub fn parse_autoconfig(xml: &[u8], email: &str) -> Option<Discovered> {
    if xml.len() > MAX_XML {
        return None;
    }
    let (local, domain) = parse_email(email)?;
    let text = std::str::from_utf8(xml).ok()?;
    let doc = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: MAX_NODES,
            ..Default::default()
        },
    )
    .ok()?;
    let root = doc.root_element();
    if !root.has_tag_name("clientConfig") {
        return None;
    }
    let address = format!("{local}@{domain}");
    let vars = [
        ("EMAILADDRESS", address.as_str()),
        ("EMAILLOCALPART", local.as_str()),
        ("EMAILDOMAIN", domain.as_str()),
    ];
    let mut out = Discovered::default();
    for provider in root.children().filter(|n| n.has_tag_name("emailProvider")) {
        for (tag, kind, slot) in [
            ("incomingServer", "imap", &mut out.imap),
            ("outgoingServer", "smtp", &mut out.smtp),
        ] {
            if slot.is_some() {
                continue;
            }
            *slot = provider
                .children()
                .filter(|n| n.has_tag_name(tag) && n.attribute("type") == Some(kind))
                .find_map(|n| server_of(n, &vars, &address));
        }
    }
    (out.imap.is_some() || out.smtp.is_some()).then_some(out)
}

fn child_text<'a>(node: roxmltree::Node<'a, '_>, tag: &str) -> Option<&'a str> {
    node.children()
        .find(|n| n.has_tag_name(tag))
        .and_then(|n| n.text())
        .map(str::trim)
}

fn server_of(node: roxmltree::Node, vars: &[(&str, &str)], address: &str) -> Option<Found> {
    let host = expand(child_text(node, "hostname")?, &vars[2..])?;
    let host = valid_hostname(&host)?;
    let port: u16 = child_text(node, "port")?.parse().ok().filter(|p| *p != 0)?;
    let security = match child_text(node, "socketType")?
        .to_ascii_uppercase()
        .as_str()
    {
        "SSL" => Security::Tls,
        "STARTTLS" => Security::StartTls,
        _ => return None,
    };
    // Komorebi signs in with a password (IMAP LOGIN, SMTP PLAIN or LOGIN):
    // a server that lists only other schemes is not for it.
    let schemes: Vec<String> = node
        .children()
        .filter(|n| n.has_tag_name("authentication"))
        .filter_map(|n| n.text())
        .map(|t| t.trim().to_ascii_lowercase())
        .collect();
    if !schemes.is_empty()
        && !schemes
            .iter()
            .any(|s| s == "password-cleartext" || s == "plain")
    {
        return None;
    }
    let user = match child_text(node, "username") {
        None | Some("") => None,
        Some(template) => {
            let user = expand(template, vars)?;
            if user.is_empty() || user.len() > 254 || user.chars().any(char::is_control) {
                return None;
            }
            (user != address).then_some(user)
        }
    };
    Some(Found {
        server: Server {
            host,
            port,
            security,
        },
        user,
    })
}

/// The records in the order a client tries them (RFC 2782): lowest priority
/// first, then the heaviest. A target of "." means the service is not
/// offered; if that is all there is, nothing is returned.
fn srv_order(records: &[SrvRecord]) -> Vec<SrvRecord> {
    let mut usable: Vec<SrvRecord> = records
        .iter()
        .filter(|r| r.port != 0 && !r.target.trim_end_matches('.').is_empty())
        .cloned()
        .collect();
    usable.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then(b.weight.cmp(&a.weight))
            .then(a.target.cmp(&b.target))
    });
    usable
}

enum Vetted {
    Ok(Found),
    /// On another domain.
    Foreign(String),
    /// Not a usable server name.
    Invalid,
}

fn vet(found: Found, domain: &str) -> Vetted {
    let host = found.server.host.clone();
    if valid_hostname(&host).is_none() || found.server.port == 0 {
        Vetted::Invalid
    } else if !same_domain(&host, domain) {
        Vetted::Foreign(host)
    } else {
        Vetted::Ok(found)
    }
}

/// The first usable server among the SRV records of `name`.
fn srv_server(
    name: &str,
    security: Security,
    domain: &str,
    lookups: &Lookups,
    refused: &mut Vec<String>,
) -> Option<Found> {
    let mut foreign = Vec::new();
    for r in srv_order(&(lookups.srv)(name)) {
        let candidate = Found {
            server: Server {
                host: valid_hostname(&r.target).unwrap_or_default(),
                port: r.port,
                security,
            },
            user: None,
        };
        match vet(candidate, domain) {
            Vetted::Ok(f) => return Some(f),
            Vetted::Foreign(h) => foreign.push(h),
            Vetted::Invalid => {}
        }
    }
    refused.extend(foreign);
    None
}

/// The servers of built-in provider `index`, compiled in; none for one
/// Komorebi cannot sign in to.
pub fn preset_servers(index: usize) -> Discovered {
    match providers::PRESETS.get(index) {
        Some(p) if p.is_supported() => Discovered {
            imap: Some(Found {
                server: p.imap_server(),
                user: None,
            }),
            smtp: Some(Found {
                server: p.smtp_server(),
                user: None,
            }),
        },
        _ => Discovered::default(),
    }
}

/// The built-in provider whose MX hosts (or, when none maps, the PTR names
/// of the first ones' addresses) say it hosts `domain`. Only an index into
/// the presets comes out of here: never a name from the answer.
fn hosted_by(domain: &str, lookups: &Lookups) -> Option<usize> {
    let mut records: Vec<MxRecord> = (lookups.mx)(domain)
        .into_iter()
        .filter(|r| valid_hostname(&r.exchange).is_some())
        .collect();
    records.sort_by(|a, b| {
        a.preference
            .cmp(&b.preference)
            .then(a.exchange.cmp(&b.exchange))
    });
    records.truncate(MX_HOSTS);
    if let Some(i) = records.iter().find_map(|r| providers::by_mx(&r.exchange)) {
        return Some(i);
    }
    records
        .iter()
        .take(PTR_HOSTS)
        .flat_map(|r| (lookups.ptr)(&r.exchange).into_iter().take(MX_HOSTS))
        .find_map(|name| providers::by_mx(&name))
}

/// Find the settings for `email`. Quiet about failure: an address Komorebi
/// cannot use, a domain with no records and an unreachable web server all
/// give an empty outcome. Makes one MX query (and, when no MX host is a
/// known provider's, address and PTR queries for at most two of them), at
/// most four SRV queries and two HTTPS requests, and none if `email` is not
/// a plain address or its domain is a built-in provider's.
pub fn discover(email: &str, lookups: &Lookups) -> Outcome {
    let mut out = Outcome::default();
    let Some((_, domain)) = parse_email(email) else {
        return out;
    };
    let by_domain = providers::guess(&format!("x@{domain}"));
    if let Some(index) = by_domain {
        out.preset = Some(index);
        out.found = preset_servers(index);
        out.sources.push(Source::Builtin);
        return out;
    }
    if let Some(index) = hosted_by(&domain, lookups) {
        out.preset = Some(index);
        out.found = preset_servers(index);
        out.sources.push(Source::Mx);
        return out;
    }

    // DNS SRV: implicit TLS first, STARTTLS only for what is still missing.
    let mut from_srv = false;
    for (tls_label, starttls_label, imap) in [
        ("_imaps._tcp", "_imap._tcp", true),
        ("_submissions._tcp", "_submission._tcp", false),
    ] {
        let mut found = None;
        for (label, security) in [
            (tls_label, Security::Tls),
            (starttls_label, Security::StartTls),
        ] {
            found = srv_server(
                &format!("{label}.{domain}"),
                security,
                &domain,
                lookups,
                &mut out.refused,
            );
            if found.is_some() {
                break;
            }
        }
        from_srv |= found.is_some();
        if imap {
            out.found.imap = found;
        } else {
            out.found.smtp = found;
        }
    }
    if from_srv {
        out.sources.push(Source::Srv);
    }

    // The provider's autoconfig file, for what SRV did not give.
    if out.found.imap.is_none() || out.found.smtp.is_none() {
        for url in [
            format!("https://autoconfig.{domain}/mail/config-v1.1.xml"),
            format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
        ] {
            let Some(config) = (lookups.fetch)(&url, &domain)
                .ok()
                .and_then(|xml| parse_autoconfig(&xml, email))
            else {
                continue;
            };
            let mut used = false;
            for (slot, found) in [
                (&mut out.found.imap, config.imap),
                (&mut out.found.smtp, config.smtp),
            ] {
                let Some(found) = found.filter(|_| slot.is_none()) else {
                    continue;
                };
                match vet(found, &domain) {
                    Vetted::Ok(f) => {
                        *slot = Some(f);
                        used = true;
                    }
                    Vetted::Foreign(h) => out.refused.push(h),
                    Vetted::Invalid => {}
                }
            }
            if used {
                out.sources.push(Source::Autoconfig);
            }
            if out.found.imap.is_some() && out.found.smtp.is_some() {
                break;
            }
        }
    }
    out.refused.sort();
    out.refused.dedup();
    // A host taken for one service is not also "refused" for another.
    let taken: Vec<&str> = [&out.found.imap, &out.found.smtp]
        .into_iter()
        .flatten()
        .map(|f| f.server.host.as_str())
        .collect();
    out.refused.retain(|h| !taken.contains(&h.as_str()));
    out
}

/// The system resolver, for SRV, MX and PTR records.
mod dns {
    use super::{MxRecord, SrvRecord};

    /// The SRV records of `name` (empty when there are none or the lookup
    /// fails). Blocks until the resolver answers: call it off the UI thread.
    pub fn query_srv(name: &str) -> Vec<SrvRecord> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::NetworkManagement::Dns::DNS_TYPE_SRV;
            // SAFETY: `query` hands `read_srv_list` a list from DnsQuery_W.
            query(name, DNS_TYPE_SRV, |list| unsafe { read_srv_list(list) })
        }
        #[cfg(not(windows))]
        {
            let _ = name;
            Vec::new()
        }
    }

    /// The MX records of `domain`, the same way.
    pub fn query_mx(domain: &str) -> Vec<MxRecord> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::NetworkManagement::Dns::DNS_TYPE_MX;
            // SAFETY: `query` hands `read_mx_list` a list from DnsQuery_W.
            query(domain, DNS_TYPE_MX, |list| unsafe { read_mx_list(list) })
        }
        #[cfg(not(windows))]
        {
            let _ = domain;
            Vec::new()
        }
    }

    /// The PTR names of a reverse name (`…in-addr.arpa`, `…ip6.arpa`), the
    /// same way.
    pub fn query_ptr(name: &str) -> Vec<String> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::NetworkManagement::Dns::DNS_TYPE_PTR;
            // SAFETY: `query` hands `read_ptr_list` a list from DnsQuery_W.
            query(name, DNS_TYPE_PTR, |list| unsafe { read_ptr_list(list) })
        }
        #[cfg(not(windows))]
        {
            let _ = name;
            Vec::new()
        }
    }

    /// One query of `kind` for `name` through `DnsQuery_W`, its answer list
    /// read by `read` and freed (empty when there is none or it fails).
    #[cfg(windows)]
    fn query<T>(
        name: &str,
        kind: u16,
        read: impl FnOnce(*const windows_sys::Win32::NetworkManagement::Dns::DNS_RECORDW) -> Vec<T>,
    ) -> Vec<T> {
        use std::ffi::c_void;
        use windows_sys::Win32::NetworkManagement::Dns::{
            DNS_QUERY_NO_MULTICAST, DNS_QUERY_TREAT_AS_FQDN, DNS_RECORDW,
        };
        use windows_sys::Win32::System::LibraryLoader::{
            GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
        };

        type QueryW = unsafe extern "system" fn(
            *const u16,
            u16,
            u32,
            *mut c_void,
            *mut *mut DNS_RECORDW,
            *mut *mut c_void,
        ) -> u32;
        type FreeFn = unsafe extern "system" fn(*const c_void, i32);

        if name.is_empty() || name.contains('\0') {
            return Vec::new();
        }
        let dll: Vec<u16> = "dnsapi.dll\0".encode_utf16().collect();
        // SAFETY: a NUL-terminated name; only the system directory is
        // searched. dnsapi.dll is loaded here, on a click, not at launch.
        let module = unsafe {
            LoadLibraryExW(
                dll.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        if module.is_null() {
            return Vec::new();
        }
        // SAFETY: the exports have these signatures (windows.h, dnsapi).
        let (query, free) = unsafe {
            (
                GetProcAddress(module, c"DnsQuery_W".as_ptr().cast()),
                GetProcAddress(module, c"DnsFree".as_ptr().cast()),
            )
        };
        let (Some(query), Some(free)) = (query, free) else {
            return Vec::new();
        };
        // SAFETY: see above; the function pointer types match the exports.
        let (query, free): (QueryW, FreeFn) = unsafe {
            (
                std::mem::transmute::<unsafe extern "system" fn() -> isize, QueryW>(query),
                std::mem::transmute::<unsafe extern "system" fn() -> isize, FreeFn>(free),
            )
        };
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let mut list: *mut DNS_RECORDW = std::ptr::null_mut();
        // SAFETY: `wide` is NUL-terminated; `list` receives the answer. The
        // name is taken as fully qualified so the search list is not
        // appended, and multicast name resolution (LLMNR, mDNS) is off.
        let status = unsafe {
            query(
                wide.as_ptr(),
                kind,
                DNS_QUERY_NO_MULTICAST | DNS_QUERY_TREAT_AS_FQDN,
                std::ptr::null_mut(),
                &mut list,
                std::ptr::null_mut(),
            )
        };
        if status != 0 || list.is_null() {
            return Vec::new();
        }
        let records = read(list.cast_const());
        // SAFETY: a list DnsQuery_W returned, freed once, after the copy.
        unsafe {
            free(list.cast(), 1 /* DnsFreeRecordList */)
        };
        records
    }

    /// A NUL-terminated wide name, read to at most 255 units.
    ///
    /// # Safety
    /// `p` must be null or point to a NUL-terminated UTF-16 string.
    #[cfg(windows)]
    unsafe fn wide_name(p: *const u16) -> String {
        let mut units = Vec::new();
        if !p.is_null() {
            for i in 0..255 {
                // SAFETY: NUL-terminated, read up to the NUL.
                let unit = unsafe { *p.add(i) };
                if unit == 0 {
                    break;
                }
                units.push(unit);
            }
        }
        String::from_utf16_lossy(&units)
    }

    /// The answer-section records of `kind` in a record list, each copied
    /// out by `each` (at most 64 records are read).
    ///
    /// # Safety
    /// `head` must be null or the head of a valid `DNS_RECORDW` list whose
    /// names are NUL-terminated; `each` is given only records of `kind`.
    #[cfg(windows)]
    unsafe fn read_list<T>(
        head: *const windows_sys::Win32::NetworkManagement::Dns::DNS_RECORDW,
        kind: u16,
        mut each: impl FnMut(&windows_sys::Win32::NetworkManagement::Dns::DNS_RECORDW) -> T,
    ) -> Vec<T> {
        let mut out = Vec::new();
        let mut at = head;
        for _ in 0..64 {
            if at.is_null() {
                break;
            }
            // SAFETY: `at` is a node of the list (checked non-null).
            let rec = unsafe { &*at };
            // The section is the low two bits of the flags: 1 is the answer.
            // SAFETY: both union views are plain integers.
            let section = unsafe { rec.Flags.DW } & 3;
            if rec.wType == kind && section == 1 {
                out.push(each(rec));
            }
            at = rec.pNext;
        }
        out
    }

    /// The answer-section SRV records of a record list, copied out.
    ///
    /// # Safety
    /// As [`read_list`].
    #[cfg(windows)]
    pub(super) unsafe fn read_srv_list(
        head: *const windows_sys::Win32::NetworkManagement::Dns::DNS_RECORDW,
    ) -> Vec<SrvRecord> {
        use windows_sys::Win32::NetworkManagement::Dns::DNS_TYPE_SRV;
        // SAFETY: the caller's list; an SRV record's data is a DNS_SRV_DATAW.
        unsafe {
            read_list(head, DNS_TYPE_SRV, |rec| {
                let srv = rec.Data.SRV;
                SrvRecord {
                    priority: srv.wPriority,
                    weight: srv.wWeight,
                    port: srv.wPort,
                    target: wide_name(srv.pNameTarget),
                }
            })
        }
    }

    /// The answer-section MX records of a record list, copied out.
    ///
    /// # Safety
    /// As [`read_list`].
    #[cfg(windows)]
    pub(super) unsafe fn read_mx_list(
        head: *const windows_sys::Win32::NetworkManagement::Dns::DNS_RECORDW,
    ) -> Vec<MxRecord> {
        use windows_sys::Win32::NetworkManagement::Dns::DNS_TYPE_MX;
        // SAFETY: the caller's list; an MX record's data is a DNS_MX_DATAW.
        unsafe {
            read_list(head, DNS_TYPE_MX, |rec| {
                let mx = rec.Data.MX;
                MxRecord {
                    preference: mx.wPreference,
                    exchange: wide_name(mx.pNameExchange),
                }
            })
        }
    }

    /// The answer-section PTR names of a record list, copied out.
    ///
    /// # Safety
    /// As [`read_list`].
    #[cfg(windows)]
    pub(super) unsafe fn read_ptr_list(
        head: *const windows_sys::Win32::NetworkManagement::Dns::DNS_RECORDW,
    ) -> Vec<String> {
        use windows_sys::Win32::NetworkManagement::Dns::DNS_TYPE_PTR;
        // SAFETY: the caller's list; a PTR record's data is a DNS_PTR_DATAW.
        unsafe { read_list(head, DNS_TYPE_PTR, |rec| wide_name(rec.Data.PTR.pNameHost)) }
    }

    #[cfg(all(test, windows))]
    mod tests {
        use super::*;
        use windows_sys::Win32::NetworkManagement::Dns::{DNS_RECORDW, DNS_TYPE_A, DNS_TYPE_SRV};

        fn wide(s: &str) -> Vec<u16> {
            s.encode_utf16().chain(Some(0)).collect()
        }

        #[test]
        fn reads_srv_answers_from_a_record_list() {
            let (a, b) = (wide("imap.example.com"), wide("mail.example.com."));
            let mut recs: Vec<DNS_RECORDW> = (0..4).map(|_| DNS_RECORDW::default()).collect();
            let set = |r: &mut DNS_RECORDW, ty, section, prio, port, name: &Vec<u16>| {
                r.wType = ty;
                r.Flags.DW = section;
                r.Data.SRV.wPriority = prio;
                r.Data.SRV.wWeight = 5;
                r.Data.SRV.wPort = port;
                r.Data.SRV.pNameTarget = name.as_ptr().cast_mut();
            };
            {
                set(&mut recs[0], DNS_TYPE_SRV, 1, 10, 993, &a);
                // An additional-section record and a non-SRV record are skipped.
                set(&mut recs[1], DNS_TYPE_SRV, 3, 1, 1, &a);
                set(&mut recs[2], DNS_TYPE_A, 1, 1, 1, &a);
                set(&mut recs[3], DNS_TYPE_SRV, 1, 20, 143, &b);
            }
            for i in 0..3 {
                let next: *mut DNS_RECORDW = &mut recs[i + 1];
                recs[i].pNext = next;
            }
            let got = unsafe { read_srv_list(recs.as_ptr()) };
            assert_eq!(
                got,
                vec![
                    SrvRecord {
                        priority: 10,
                        weight: 5,
                        port: 993,
                        target: "imap.example.com".into()
                    },
                    SrvRecord {
                        priority: 20,
                        weight: 5,
                        port: 143,
                        target: "mail.example.com.".into()
                    },
                ]
            );
            assert!(unsafe { read_srv_list(std::ptr::null()) }.is_empty());
        }

        #[test]
        fn reads_mx_and_ptr_answers_from_a_record_list() {
            use windows_sys::Win32::NetworkManagement::Dns::{DNS_TYPE_MX, DNS_TYPE_PTR};
            let (a, b, p) = (
                wide("mail.postale.io."),
                wide("backup.example.net"),
                wide("host.mail.protection.outlook.com."),
            );
            let mut recs: Vec<DNS_RECORDW> = (0..5).map(|_| DNS_RECORDW::default()).collect();
            let mx = |r: &mut DNS_RECORDW, section, pref, name: &Vec<u16>| {
                r.wType = DNS_TYPE_MX;
                r.Flags.DW = section;
                r.Data.MX.wPreference = pref;
                r.Data.MX.pNameExchange = name.as_ptr().cast_mut();
            };
            mx(&mut recs[0], 1, 10, &a);
            // An additional-section MX record is skipped.
            mx(&mut recs[1], 3, 0, &b);
            mx(&mut recs[2], 1, 20, &b);
            recs[3].wType = DNS_TYPE_PTR;
            recs[3].Flags.DW = 1;
            recs[3].Data.PTR.pNameHost = p.as_ptr().cast_mut();
            // A PTR record with no name reads as empty.
            recs[4].wType = DNS_TYPE_PTR;
            recs[4].Flags.DW = 1;
            for i in 0..4 {
                let next: *mut DNS_RECORDW = &mut recs[i + 1];
                recs[i].pNext = next;
            }
            let got = unsafe { read_mx_list(recs.as_ptr()) };
            assert_eq!(
                got,
                vec![
                    MxRecord {
                        preference: 10,
                        exchange: "mail.postale.io.".into()
                    },
                    MxRecord {
                        preference: 20,
                        exchange: "backup.example.net".into()
                    },
                ]
            );
            let names = unsafe { read_ptr_list(recs.as_ptr()) };
            assert_eq!(names, ["host.mail.protection.outlook.com.", ""]);
            assert!(unsafe { read_mx_list(std::ptr::null()) }.is_empty());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const CONFIG: &str = r#"<?xml version="1.0"?>
<clientConfig version="1.1">
  <emailProvider id="example.com">
    <domain>example.com</domain>
    <incomingServer type="imap">
      <hostname>imap.example.com</hostname>
      <port>993</port>
      <socketType>SSL</socketType>
      <username>%EMAILLOCALPART%</username>
      <authentication>password-cleartext</authentication>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>smtp.example.com</hostname>
      <port>587</port>
      <socketType>STARTTLS</socketType>
      <username>%EMAILADDRESS%</username>
      <authentication>password-cleartext</authentication>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#;

    fn tls(host: &str, port: u16) -> Server {
        Server {
            host: host.into(),
            port,
            security: Security::Tls,
        }
    }

    fn starttls(host: &str, port: u16) -> Server {
        Server {
            host: host.into(),
            port,
            security: Security::StartTls,
        }
    }

    fn rec(priority: u16, weight: u16, port: u16, target: &str) -> SrvRecord {
        SrvRecord {
            priority,
            weight,
            port,
            target: target.into(),
        }
    }

    /// Lookups answering from tables, counting the requests made.
    struct Fake {
        mx: Vec<(String, Vec<MxRecord>)>,
        ptr: Vec<(String, Vec<String>)>,
        srv: Vec<(String, Vec<SrvRecord>)>,
        web: Vec<(String, Vec<u8>)>,
        mx_calls: RefCell<Vec<String>>,
        ptr_calls: RefCell<Vec<String>>,
        srv_calls: RefCell<Vec<String>>,
        fetch_calls: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new() -> Self {
            Fake {
                mx: Vec::new(),
                ptr: Vec::new(),
                srv: Vec::new(),
                web: Vec::new(),
                mx_calls: RefCell::default(),
                ptr_calls: RefCell::default(),
                srv_calls: RefCell::default(),
                fetch_calls: RefCell::default(),
            }
        }
        fn with_mx(mut self, domain: &str, hosts: &[(u16, &str)]) -> Self {
            self.mx.push((
                domain.into(),
                hosts
                    .iter()
                    .map(|(preference, h)| MxRecord {
                        preference: *preference,
                        exchange: (*h).into(),
                    })
                    .collect(),
            ));
            self
        }
        fn run(&self, email: &str) -> Outcome {
            let mx = |name: &str| {
                self.mx_calls.borrow_mut().push(name.to_string());
                self.mx
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, r)| r.clone())
                    .unwrap_or_default()
            };
            let ptr = |name: &str| {
                self.ptr_calls.borrow_mut().push(name.to_string());
                self.ptr
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, r)| r.clone())
                    .unwrap_or_default()
            };
            let srv = |name: &str| {
                self.srv_calls.borrow_mut().push(name.to_string());
                self.srv
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, r)| r.clone())
                    .unwrap_or_default()
            };
            let fetch = |url: &str, _domain: &str| {
                self.fetch_calls.borrow_mut().push(url.to_string());
                self.web
                    .iter()
                    .find(|(u, _)| u == url)
                    .map(|(_, b)| b.clone())
                    .ok_or(FetchError::Status(404))
            };
            discover(
                email,
                &Lookups {
                    mx: &mx,
                    ptr: &ptr,
                    srv: &srv,
                    fetch: &fetch,
                },
            )
        }
    }

    fn preset(id: &str) -> usize {
        providers::index_of(id).unwrap()
    }

    #[test]
    fn a_custom_domain_whose_mx_is_postale_is_postale() {
        let f = Fake::new().with_mx("owner.example", &[(0, "mail.postale.io.")]);
        let out = f.run("me@owner.example");
        assert_eq!(out.preset, Some(preset("postale")));
        assert_eq!(out.sources, vec![Source::Mx]);
        // The preset's own servers, never the domain as the server.
        assert_eq!(out.found.imap.unwrap().server, tls("mail.postale.io", 993));
        assert_eq!(out.found.smtp.unwrap().server, tls("mail.postale.io", 465));
        // MX decided it: no SRV, PTR or web lookups.
        assert_eq!(*f.mx_calls.borrow(), vec!["owner.example".to_string()]);
        assert!(f.srv_calls.borrow().is_empty() && f.fetch_calls.borrow().is_empty());
        assert!(f.ptr_calls.borrow().is_empty());
    }

    #[test]
    fn a_custom_domain_on_google_mx_is_google_workspace() {
        let f = Fake::new().with_mx(
            "company.example",
            &[
                (5, "alt1.aspmx.l.google.com."),
                (1, "ASPMX.L.GOOGLE.COM."),
                (10, "aspmx2.googlemail.com."),
            ],
        );
        let out = f.run("ana@company.example");
        assert_eq!(out.preset, Some(preset("google-workspace")));
        assert_eq!(out.found.imap.unwrap().server, tls("imap.gmail.com", 993));
        assert_eq!(out.sources, vec![Source::Mx]);
    }

    #[test]
    fn microsoft_365_mx_names_the_provider_but_fills_nothing() {
        let f = Fake::new().with_mx(
            "corp.example",
            &[(0, "corp-example.mail.protection.outlook.com")],
        );
        let out = f.run("ana@corp.example");
        assert_eq!(out.preset, Some(preset("microsoft365")));
        assert!(out.is_empty());
        assert!(f.srv_calls.borrow().is_empty());
    }

    #[test]
    fn an_unknown_mx_falls_through_to_srv() {
        let mut f = Fake::new().with_mx("example.com", &[(10, "mx.hosting.example.net")]);
        f.srv.push((
            "_imaps._tcp.example.com".into(),
            vec![rec(0, 0, 993, "imap.example.com")],
        ));
        let out = f.run("me@example.com");
        assert_eq!(out.preset, None);
        assert_eq!(out.found.imap.unwrap().server, tls("imap.example.com", 993));
        assert_eq!(out.sources, vec![Source::Srv]);
        // The PTR names of the unknown MX host were asked for, matched
        // nothing, and the MX host is in no answer.
        assert_eq!(
            *f.ptr_calls.borrow(),
            vec!["mx.hosting.example.net".to_string()]
        );
        assert!(!out.refused.iter().any(|h| h.contains("hosting")));
    }

    #[test]
    fn the_ptr_of_an_unknown_mx_host_can_name_a_preset() {
        let mut f = Fake::new().with_mx(
            "shop.example",
            &[(20, "mx2.shop.example"), (10, "mx1.shop.example")],
        );
        f.ptr
            .push(("mx1.shop.example".into(), vec!["mail.postale.io.".into()]));
        let out = f.run("me@shop.example");
        assert_eq!(out.preset, Some(preset("postale")));
        assert_eq!(out.sources, vec![Source::Mx]);
        // Lowest preference first; it answered, so the second was not asked.
        assert_eq!(*f.ptr_calls.borrow(), vec!["mx1.shop.example".to_string()]);
    }

    #[test]
    fn a_hostile_mx_answer_never_puts_its_own_host_in_the_form() {
        // Every MX and PTR name an attacker could answer with: lookalikes,
        // a preset name as a prefix, addresses, junk and many records.
        let mut hosts: Vec<(u16, String)> = vec![
            (0, "mail.postale.io.evil.example".into()),
            (0, "evilpostale.io".into()),
            (0, "postale.io-mail.evil.example".into()),
            (0, "192.0.2.1".into()),
            (0, "mail.evil.example:993".into()),
            (0, "".into()),
            (0, ".".into()),
        ];
        hosts.extend((0..100).map(|i| (1, format!("mx{i}.evil.example"))));
        let refs: Vec<(u16, &str)> = hosts.iter().map(|(p, h)| (*p, h.as_str())).collect();
        let mut f = Fake::new().with_mx("victim.example", &refs);
        f.ptr.push((
            "evilpostale.io".into(),
            vec!["imap.evil.example".into(), "postale.io.evil.example".into()],
        ));
        let out = f.run("me@victim.example");
        assert_eq!(out.preset, None);
        assert!(out.is_empty());
        let named: Vec<String> = [&out.found.imap, &out.found.smtp]
            .into_iter()
            .flatten()
            .map(|f| f.server.host.clone())
            .chain(out.refused.iter().cloned())
            .collect();
        assert!(named.iter().all(|h| !h.contains("evil")), "{named:?}");
        // At most two hosts' PTR names are asked for.
        assert!(f.ptr_calls.borrow().len() <= PTR_HOSTS);

        // And when an MX does select a preset, the form gets the compiled-in
        // servers whatever the answer's own name was.
        for host in ["x.mail.postale.io", "MAIL.POSTALE.IO."] {
            let f = Fake::new().with_mx("victim.example", &[(0, host)]);
            let out = f.run("me@victim.example");
            for found in [out.found.imap.unwrap(), out.found.smtp.unwrap()] {
                assert_eq!(found.server.host, "mail.postale.io");
            }
        }
        // Every preset's servers are its own, compiled in.
        for (i, p) in providers::PRESETS.iter().enumerate() {
            let servers = preset_servers(i);
            if p.is_supported() {
                assert_eq!(servers.imap.unwrap().server, p.imap_server());
            } else {
                assert!(servers.imap.is_none() && servers.smtp.is_none());
            }
        }
    }

    #[test]
    fn reverse_names() {
        assert_eq!(
            reverse_name("192.0.2.10".parse().unwrap()),
            "10.2.0.192.in-addr.arpa"
        );
        assert_eq!(
            reverse_name("2001:db8::1".parse().unwrap()),
            "1.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.8.b.d.0.1.0.0.2.ip6.arpa"
        );
    }

    #[test]
    fn autoconfig_maps_servers_and_placeholders() {
        let d = parse_autoconfig(CONFIG.as_bytes(), "Alice@Example.com").unwrap();
        assert_eq!(
            d.imap,
            Some(Found {
                server: tls("imap.example.com", 993),
                // The local part differs from the address.
                user: Some("Alice".into()),
            })
        );
        assert_eq!(
            d.smtp,
            Some(Found {
                server: starttls("smtp.example.com", 587),
                // The full address is what the account uses anyway.
                user: None,
            })
        );
    }

    #[test]
    fn autoconfig_ignores_plaintext_and_foreign_schemes() {
        let xml = r#"<clientConfig><emailProvider>
          <incomingServer type="imap"><hostname>plain.example.com</hostname><port>143</port><socketType>plain</socketType></incomingServer>
          <incomingServer type="imap"><hostname>oauth.example.com</hostname><port>993</port><socketType>SSL</socketType><authentication>oauth2</authentication></incomingServer>
          <incomingServer type="imap"><hostname>crypt.example.com</hostname><port>993</port><socketType>SSL</socketType><authentication>password-encrypted</authentication></incomingServer>
          <incomingServer type="pop3"><hostname>pop.example.com</hostname><port>995</port><socketType>SSL</socketType></incomingServer>
          <incomingServer type="imap"><hostname>imap.example.com</hostname><port>143</port><socketType>starttls</socketType><authentication>oauth2</authentication><authentication>password-cleartext</authentication></incomingServer>
          <outgoingServer type="smtp"><hostname>smtp.example.com</hostname><port>25</port><socketType>plain</socketType></outgoingServer>
        </emailProvider></clientConfig>"#;
        let d = parse_autoconfig(xml.as_bytes(), "a@example.com").unwrap();
        assert_eq!(d.imap.unwrap().server, starttls("imap.example.com", 143));
        assert_eq!(d.smtp, None);
    }

    fn one(server: &str) -> String {
        format!("<clientConfig><emailProvider>{server}</emailProvider></clientConfig>")
    }

    #[test]
    fn autoconfig_rejects_bad_ports_hosts_and_placeholders() {
        let imap = |inner: &str| {
            one(&format!(
                "<incomingServer type=\"imap\">{inner}</incomingServer>"
            ))
        };
        for (what, inner) in [
            (
                "port 0",
                "<hostname>h.example.com</hostname><port>0</port><socketType>SSL</socketType>",
            ),
            (
                "port 65536",
                "<hostname>h.example.com</hostname><port>65536</port><socketType>SSL</socketType>",
            ),
            (
                "no port",
                "<hostname>h.example.com</hostname><socketType>SSL</socketType>",
            ),
            (
                "negative port",
                "<hostname>h.example.com</hostname><port>-1</port><socketType>SSL</socketType>",
            ),
            (
                "ip host",
                "<hostname>192.0.2.1</hostname><port>993</port><socketType>SSL</socketType>",
            ),
            (
                "url host",
                "<hostname>https://h.example.com/</hostname><port>993</port><socketType>SSL</socketType>",
            ),
            (
                "unknown placeholder in host",
                "<hostname>%EMAILLOCALPART%.example.com</hostname><port>993</port><socketType>SSL</socketType>",
            ),
            (
                "unknown placeholder in user",
                "<hostname>h.example.com</hostname><port>993</port><socketType>SSL</socketType><username>%FOO%</username>",
            ),
            (
                "stray percent in user",
                "<hostname>h.example.com</hostname><port>993</port><socketType>SSL</socketType><username>50%</username>",
            ),
            (
                "control character in user",
                "<hostname>h.example.com</hostname><port>993</port><socketType>SSL</socketType><username>a&#1;b</username>",
            ),
        ] {
            assert_eq!(
                parse_autoconfig(imap(inner).as_bytes(), "a@example.com"),
                None,
                "{what}"
            );
        }
    }

    #[test]
    fn placeholders_with_odd_characters_are_not_expanded_twice() {
        // `%` is legal in a local part; the value is inserted, not rescanned.
        let xml = one(
            "<incomingServer type=\"imap\"><hostname>imap.%EMAILDOMAIN%</hostname>\
             <port>993</port><socketType>SSL</socketType>\
             <username>%EMAILLOCALPART%</username></incomingServer>",
        );
        let d = parse_autoconfig(xml.as_bytes(), "50%off+x@example.com").unwrap();
        let imap = d.imap.unwrap();
        assert_eq!(imap.server.host, "imap.example.com");
        assert_eq!(imap.user.as_deref(), Some("50%off+x"));
        // Not an address Find settings can use.
        for bad in [
            "a b@example.com",
            "a\"b@example.com",
            "a@",
            "@example.com",
            "a@localhost",
            "a@exa mple.com",
            "a@[192.0.2.1]",
            "a@192.0.2.1",
            "a..b@example.com",
            "",
            "plain",
        ] {
            assert!(!looks_valid(bad), "{bad:?}");
            assert_eq!(parse_autoconfig(xml.as_bytes(), bad), None, "{bad:?}");
        }
        assert!(looks_valid("  a.b+c@Example.COM  "));
        assert!(looks_valid("me@exämple.com"));
    }

    #[test]
    fn hostile_xml_is_refused() {
        // A DTD, with or without entities (billion laughs, external).
        let laughs = r#"<?xml version="1.0"?>
<!DOCTYPE clientConfig [
 <!ENTITY a "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa">
 <!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;">
 <!ENTITY c "&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;">
]>
<clientConfig><emailProvider><incomingServer type="imap"><hostname>&c;.example.com</hostname><port>993</port><socketType>SSL</socketType></incomingServer></emailProvider></clientConfig>"#;
        assert_eq!(parse_autoconfig(laughs.as_bytes(), "a@example.com"), None);
        let external = r#"<?xml version="1.0"?>
<!DOCTYPE x [ <!ENTITY e SYSTEM "file:///c:/windows/win.ini"> ]>
<clientConfig><emailProvider><incomingServer type="imap"><hostname>&e;</hostname><port>993</port><socketType>SSL</socketType></incomingServer></emailProvider></clientConfig>"#;
        assert_eq!(parse_autoconfig(external.as_bytes(), "a@example.com"), None);
        // Even an otherwise good file with an (empty) DTD is not read twice.
        let with_dtd = format!("<!DOCTYPE clientConfig [ <!ENTITY x \"y\"> ]>{CONFIG}");
        assert_eq!(parse_autoconfig(with_dtd.as_bytes(), "a@example.com"), None);

        // Too large.
        let big = format!("{CONFIG}<!--{}-->", "x".repeat(MAX_XML));
        assert_eq!(parse_autoconfig(big.as_bytes(), "a@example.com"), None);
        // Too many nodes (small but deep and wide).
        let wide = format!(
            "<clientConfig><emailProvider>{}</emailProvider></clientConfig>",
            "<a/>".repeat(MAX_NODES as usize + 10)
        );
        assert!(wide.len() < MAX_XML);
        assert_eq!(parse_autoconfig(wide.as_bytes(), "a@example.com"), None);
        // Not XML, wrong root, wrong encoding, malformed.
        for junk in [
            &b"not xml"[..],
            b"<html><body>Not found</body></html>",
            b"<clientConfig><emailProvider>",
            b"\xff\xfe<\0c\0",
            b"",
        ] {
            assert_eq!(parse_autoconfig(junk, "a@example.com"), None);
        }
    }

    #[test]
    fn registrable_domains() {
        for (host, domain, same) in [
            ("imap.example.com", "example.com", true),
            ("example.com", "example.com", true),
            ("a.b.example.com", "mail.example.com", true),
            ("IMAP.Example.COM.", "example.com", true),
            ("mail.other.net", "example.com", false),
            ("example.com.evil.net", "example.com", false),
            ("evilexample.com", "example.com", false),
            ("imap.example.co.uk", "example.co.uk", true),
            ("imap.other.co.uk", "example.co.uk", false),
            ("co.uk", "example.co.uk", false),
            ("mail.me.duckdns.org", "me.duckdns.org", true),
            ("mail.you.duckdns.org", "me.duckdns.org", false),
            ("192.0.2.1", "example.com", false),
            ("", "example.com", false),
            ("a b.example.com", "example.com", false),
        ] {
            assert_eq!(same_domain(host, domain), same, "{host} / {domain}");
        }
    }

    #[test]
    fn srv_prefers_implicit_tls_and_orders_by_priority_then_weight() {
        let mut f = Fake::new();
        f.srv.push((
            "_imaps._tcp.example.com".into(),
            vec![
                rec(20, 100, 993, "backup.example.com."),
                rec(10, 10, 993, "light.example.com"),
                rec(10, 60, 9930, "heavy.example.com"),
            ],
        ));
        f.srv.push((
            "_submissions._tcp.example.com".into(),
            vec![rec(0, 1, 465, "smtp.example.com")],
        ));
        let out = f.run("me@example.com");
        assert_eq!(
            out.found.imap.unwrap().server,
            tls("heavy.example.com", 9930)
        );
        assert_eq!(out.found.smtp.unwrap().server, tls("smtp.example.com", 465));
        assert_eq!(out.sources, vec![Source::Srv]);
        // Both found by SRV: no web request, no STARTTLS queries.
        assert!(f.fetch_calls.borrow().is_empty());
        assert_eq!(f.srv_calls.borrow().len(), 2);
    }

    #[test]
    fn srv_falls_back_to_starttls_and_a_dot_target_means_unavailable() {
        let mut f = Fake::new();
        f.srv
            .push(("_imaps._tcp.example.com".into(), vec![rec(0, 0, 993, ".")]));
        f.srv.push((
            "_imap._tcp.example.com".into(),
            vec![rec(5, 0, 143, "imap.example.com")],
        ));
        f.srv.push((
            "_submission._tcp.example.com".into(),
            vec![rec(0, 0, 587, "."), rec(1, 0, 2525, "smtp.example.com")],
        ));
        let out = f.run("me@example.com");
        assert_eq!(
            out.found.imap.unwrap().server,
            starttls("imap.example.com", 143)
        );
        // The "." record is skipped, the next one used.
        assert_eq!(
            out.found.smtp.unwrap().server,
            starttls("smtp.example.com", 2525)
        );
        assert_eq!(f.srv_calls.borrow().len(), 4);

        // Only "." records: nothing is offered.
        let mut f = Fake::new();
        for name in [
            "_imaps._tcp.example.com",
            "_imap._tcp.example.com",
            "_submissions._tcp.example.com",
            "_submission._tcp.example.com",
        ] {
            f.srv.push((name.into(), vec![rec(0, 0, 0, ".")]));
        }
        let out = f.run("me@example.com");
        assert!(out.is_empty());
        assert!(out.sources.is_empty());
    }

    #[test]
    fn srv_to_another_domain_is_reported_not_taken() {
        let mut f = Fake::new();
        f.srv.push((
            "_imaps._tcp.example.com".into(),
            vec![rec(0, 0, 993, "mail.hosting.net")],
        ));
        let out = f.run("me@example.com");
        assert_eq!(out.found.imap, None);
        assert_eq!(out.refused, vec!["mail.hosting.net".to_string()]);
    }

    #[test]
    fn autoconfig_fills_what_srv_did_not_and_takes_two_urls_at_most() {
        let mut f = Fake::new();
        f.srv.push((
            "_imaps._tcp.example.com".into(),
            vec![rec(0, 0, 993, "srv-imap.example.com")],
        ));
        f.web.push((
            "https://autoconfig.example.com/mail/config-v1.1.xml".into(),
            CONFIG.as_bytes().to_vec(),
        ));
        let out = f.run("me@example.com");
        // SRV wins for IMAP; autoconfig supplies SMTP.
        assert_eq!(
            out.found.imap.unwrap().server,
            tls("srv-imap.example.com", 993)
        );
        assert_eq!(
            out.found.smtp.unwrap().server,
            starttls("smtp.example.com", 587)
        );
        assert_eq!(out.sources, vec![Source::Srv, Source::Autoconfig]);
        // The URL has the domain only, never the address.
        assert_eq!(
            *f.fetch_calls.borrow(),
            vec!["https://autoconfig.example.com/mail/config-v1.1.xml".to_string()]
        );

        // Nothing anywhere: both URLs once, quiet empty result.
        let f = Fake::new();
        let out = f.run("me@example.com");
        assert!(out.is_empty() && out.refused.is_empty());
        assert_eq!(
            *f.fetch_calls.borrow(),
            vec![
                "https://autoconfig.example.com/mail/config-v1.1.xml".to_string(),
                "https://example.com/.well-known/autoconfig/mail/config-v1.1.xml".to_string(),
            ]
        );
        for url in f.fetch_calls.borrow().iter() {
            assert!(!url.contains('?') && !url.contains("me@") && !url.contains("emailaddress"));
        }
    }

    #[test]
    fn well_known_is_tried_after_the_autoconfig_host() {
        let mut f = Fake::new();
        f.web.push((
            "https://example.com/.well-known/autoconfig/mail/config-v1.1.xml".into(),
            CONFIG.as_bytes().to_vec(),
        ));
        let out = f.run("me@example.com");
        assert_eq!(out.found.imap.unwrap().server, tls("imap.example.com", 993));
        assert_eq!(out.sources, vec![Source::Autoconfig]);
        assert_eq!(f.fetch_calls.borrow().len(), 2);
    }

    #[test]
    fn cross_domain_autoconfig_is_not_prefilled() {
        let xml = CONFIG.replace("imap.example.com", "imap.other.net");
        let mut f = Fake::new();
        f.web.push((
            "https://autoconfig.example.com/mail/config-v1.1.xml".into(),
            xml.into_bytes(),
        ));
        let out = f.run("me@example.com");
        assert_eq!(out.found.imap, None);
        assert_eq!(out.found.smtp.unwrap().server.host, "smtp.example.com");
        assert_eq!(out.refused, vec!["imap.other.net".to_string()]);
    }

    #[test]
    fn builtin_providers_need_no_lookup() {
        let f = Fake::new();
        let out = f.run("me@postale.io");
        assert_eq!(out.sources, vec![Source::Builtin]);
        assert_eq!(out.preset, Some(preset("postale")));
        assert_eq!(out.found.imap.unwrap().server, tls("mail.postale.io", 993));
        assert_eq!(out.found.smtp.unwrap().server, tls("mail.postale.io", 465));
        assert!(f.srv_calls.borrow().is_empty() && f.fetch_calls.borrow().is_empty());
        assert!(f.mx_calls.borrow().is_empty());
        // A provider Komorebi cannot sign in to is named, its servers not.
        let out = f.run("me@hotmail.com");
        assert_eq!(out.preset, Some(preset("outlook")));
        assert!(out.is_empty());
    }

    #[test]
    fn an_unusable_address_makes_no_request() {
        let f = Fake::new();
        for email in ["", "nobody", "a@localhost", "a@192.0.2.1"] {
            assert!(f.run(email).is_empty());
        }
        assert!(f.srv_calls.borrow().is_empty() && f.fetch_calls.borrow().is_empty());
        assert!(f.mx_calls.borrow().is_empty());
    }

    #[test]
    fn redirects_stay_on_the_email_domain_over_https() {
        let ok = |u: &str| redirect_allowed(&url::Url::parse(u).unwrap(), "example.com");
        assert!(ok("https://www.example.com/autoconfig.xml"));
        assert!(ok("https://example.com/x"));
        assert!(!ok("https://other.net/x"));
        assert!(!ok("https://example.com.other.net/x"));
        assert!(!ok("http://www.example.com/x"));
        assert!(!ok("https://192.0.2.1/x"));
        assert!(!ok("https://[::1]/x"));
    }
}
