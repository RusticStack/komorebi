//! Certificate pinning for self-hosted IMAP and SMTP servers.
//!
//! A server whose certificate Windows does not trust (self-signed, or from
//! a private CA) can be trusted explicitly: the user reviews the certificate
//! once ([`probe_certificate`]), confirms its SHA-256 fingerprint, and the
//! DER of that one certificate is saved with the account (`imap_pin`,
//! `smtp_pin`). It is the identity of one server on one host and port, never
//! offered to another.
//!
//! # Spike result (SChannel, native-tls 0.2.18, schannel 0.1.29)
//!
//! Checked by the tests below on this machine's SChannel:
//!
//! * A certificate added with `add_root_certificate` is trusted as an anchor
//!   when it is the presented certificate: a self-signed leaf with or
//!   without `CA:TRUE`, and a leaf issued by a private CA that was not sent.
//!   Any other certificate for the same names is refused (`CERT_E_UNTRUSTEDROOT`).
//! * `disable_built_in_roots(true)` is honoured: native-tls installs a verify
//!   callback that, after Windows' own verdict (which allows an unknown CA
//!   only when a certificate of the chain is in the pin store), requires
//!   that the chain contains a pinned certificate. A certificate that
//!   Windows trusts through the system roots, for another host, therefore
//!   fails a pinned connection. (There is no offline way to mint such a
//!   certificate, so this part is read from the source, not tested; the IMAP
//!   path adds a byte-for-byte comparison anyway, see [`check_peer`].)
//! * The host name check is relaxed on a pinned connector only, because the
//!   pin names the server. IP literals and `localhost` work alike.
//! * Revocation: schannel builds the chain with
//!   `CERT_CHAIN_REVOCATION_CHECK_CACHE_ONLY`, so Windows consults only its
//!   cache for CRL/OCSP and does not fetch revocation lists. Chain building
//!   may still fetch a missing intermediate certificate from the URL the
//!   certificate names (AIA); that happens with or without a pin and is
//!   described in PRIVACY.md.
//! * SMTP (lettre) takes the same connector settings through
//!   `TlsParameters::builder`: `add_root_certificate`,
//!   `CertificateStore::None` (the same `disable_built_in_roots`) and
//!   relaxed host names. lettre exposes no peer certificate, so SMTP relies
//!   on the connector alone.
//!
//! Nothing here accepts an invalid certificate on a connection that goes
//! on to send credentials. The one accept-everything handshake is the probe,
//! which sends nothing and reads one certificate.

use crate::providers::Security;
use anyhow::{Context, Result, anyhow, bail};
use native_tls::{Certificate, TlsConnector, TlsStream};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, ToSocketAddrs};
use std::time::Duration;

/// The longest pinned certificate kept or read (DER bytes).
pub const MAX_PIN: usize = 16 * 1024;

/// Which of an account's two servers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Imap,
    Smtp,
}

/// What a saved pin trusts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    /// This exact certificate, byte for byte (the DER is the server's).
    Certificate,
    /// The server's key: any certificate with the same SubjectPublicKeyInfo
    /// (the DER is the server's certificate it was read from; its key's
    /// SHA-256 is compared after the handshake).
    Key,
    /// The issuing CA: any certificate for this host that the saved CA
    /// certificate signed (the DER is the CA's; Windows checks the chain
    /// and the host name, with no other roots).
    Ca,
}

/// A usable pin: what it trusts and the certificate it keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pin<'a> {
    pub trust: Trust,
    pub der: &'a [u8],
}

/// The saved form of a key or CA pin: this tag, a byte for the kind, and
/// the certificate. A bare certificate (the first form, starting `0x30`)
/// is a certificate pin. A build that predates the tag reads a tagged pin
/// as damaged and lets Windows validate, so a downgrade fails safe.
const TAG: &[u8] = b"KPIN";

/// The bytes to save for `trust` of `der`.
pub fn encode(trust: Trust, der: &[u8]) -> Vec<u8> {
    let kind = match trust {
        Trust::Certificate => return der.to_vec(),
        Trust::Key => 1,
        Trust::Ca => 2,
    };
    let mut out = Vec::with_capacity(TAG.len() + 1 + der.len());
    out.extend_from_slice(TAG);
    out.push(kind);
    out.extend_from_slice(der);
    out
}

/// What `blob` trusts and its certificate, before checking the certificate.
fn decode(blob: &[u8]) -> Option<Pin<'_>> {
    match blob.strip_prefix(TAG) {
        None => Some(Pin {
            trust: Trust::Certificate,
            der: blob,
        }),
        Some([1, der @ ..]) => Some(Pin {
            trust: Trust::Key,
            der,
        }),
        Some([2, der @ ..]) => Some(Pin {
            trust: Trust::Ca,
            der,
        }),
        Some(_) => None,
    }
}

/// A saved pin, as it can be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinState<'a> {
    /// Nothing saved: Windows validates the certificate.
    None,
    Pinned(Pin<'a>),
    /// Something is saved but it is empty, too long or not a certificate.
    /// Never trusted: the server is validated by Windows, and the form says
    /// the saved certificate needs review.
    Damaged,
}

pub fn state(blob: Option<&[u8]>) -> PinState<'_> {
    let Some(blob) = blob else {
        return PinState::None;
    };
    match decode(blob) {
        Some(pin)
            if !pin.der.is_empty()
                && pin.der.len() <= MAX_PIN
                && Certificate::from_der(pin.der).is_ok()
                && describe(pin.der).is_some()
                && super::x509::spki(pin.der).is_some() =>
        {
            PinState::Pinned(pin)
        }
        _ => PinState::Damaged,
    }
}

/// The pin to connect with: `blob` if it holds a good certificate.
pub fn usable(blob: Option<&[u8]>) -> Option<Pin<'_>> {
    match state(blob) {
        PinState::Pinned(pin) => Some(pin),
        PinState::None | PinState::Damaged => None,
    }
}

/// The server's certificate differed from the saved one. Wrapped in the
/// error chain, behind the handshake failure it is reported as.
#[derive(Debug)]
pub struct PinMismatch(pub String);

impl std::fmt::Display for PinMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "the certificate of {} is not the saved one", self.0)
    }
}

impl std::error::Error for PinMismatch {}

/// The connector for one server. Without a usable `pin`, the default
/// validation. A certificate pin is the only root (never the Windows
/// roots) and the host name is not compared; a CA pin is the only root and
/// the host name is checked (a CA signs for many hosts); a key pin accepts
/// any certificate in the handshake, so it is only sound followed by
/// [`check_peer`] before anything is sent (the IMAP connect does both).
pub fn connector(pin: Option<&[u8]>) -> Result<TlsConnector> {
    let mut builder = TlsConnector::builder();
    match usable(pin) {
        None => {}
        Some(Pin {
            trust: Trust::Certificate,
            der,
        }) => {
            builder
                .add_root_certificate(
                    Certificate::from_der(der).context("reading the saved certificate")?,
                )
                .disable_built_in_roots(true)
                .danger_accept_invalid_hostnames(true);
        }
        Some(Pin {
            trust: Trust::Ca,
            der,
        }) => {
            builder
                .add_root_certificate(
                    Certificate::from_der(der).context("reading the saved certificate")?,
                )
                .disable_built_in_roots(true);
        }
        Some(Pin {
            trust: Trust::Key, ..
        }) => {
            builder
                .danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true);
        }
    }
    builder.build().context("initializing TLS")
}

/// SMTP's TLS parameters for the same cases; the key pin is only sound
/// followed by [`check_presented`] before AUTH (`smtp::session` does both).
pub fn smtp_tls_parameters(
    host: &str,
    pin: Option<&[u8]>,
) -> Result<lettre::transport::smtp::client::TlsParameters> {
    use lettre::transport::smtp::client::{Certificate as Cert, CertificateStore, TlsParameters};
    let mut builder = TlsParameters::builder(host.to_string());
    match usable(pin) {
        None => {}
        Some(Pin {
            trust: Trust::Certificate,
            der,
        }) => {
            builder = builder
                .add_root_certificate(Cert::from_der(der.to_vec())?)
                .certificate_store(CertificateStore::None)
                .dangerous_accept_invalid_hostnames(true);
        }
        Some(Pin {
            trust: Trust::Ca,
            der,
        }) => {
            builder = builder
                .add_root_certificate(Cert::from_der(der.to_vec())?)
                .certificate_store(CertificateStore::None);
        }
        Some(Pin {
            trust: Trust::Key, ..
        }) => {
            builder = builder
                .dangerous_accept_invalid_certs(true)
                .dangerous_accept_invalid_hostnames(true);
        }
    }
    Ok(builder.build_native()?)
}

/// After the handshake of a pinned connection, before anything else is
/// sent: the certificate the server presented (`presented`, its DER) must
/// be the saved one byte for byte, or carry the saved key. A CA pin was
/// checked by Windows against the CA alone. (The connector already refused
/// other certificates for a certificate pin; this does not rely on that.)
pub fn check_presented(presented: Option<&[u8]>, pin: Option<&[u8]>, host: &str) -> Result<()> {
    let ok = match usable(pin) {
        None => true,
        Some(Pin {
            trust: Trust::Certificate,
            der,
        }) => presented == Some(der),
        Some(Pin {
            trust: Trust::Key,
            der,
        }) => {
            let saved = super::x509::spki_sha256(der);
            saved.is_some() && presented.and_then(super::x509::spki_sha256) == saved
        }
        Some(Pin {
            trust: Trust::Ca, ..
        }) => presented.is_some(),
    };
    if ok {
        Ok(())
    } else {
        Err(PinMismatch(host.to_string()).into())
    }
}

/// [`check_presented`] on a TLS stream.
pub fn check_peer<S: Read + Write>(
    stream: &TlsStream<S>,
    pin: Option<&[u8]>,
    host: &str,
) -> Result<()> {
    if usable(pin).is_none() {
        return Ok(());
    }
    let presented = stream
        .peer_certificate()
        .ok()
        .flatten()
        .and_then(|c| c.to_der().ok());
    check_presented(presented.as_deref(), pin, host)
}

// ---- the probe ---------------------------------------------------------

/// The longest server line read before the handshake.
const MAX_LINE: usize = 1024;
/// The most lines of one reply (a long EHLO).
const MAX_LINES: usize = 64;
/// How long one read or write may take before the handshake.
const PROBE_IO: Duration = Duration::from_secs(10);

/// Read one line of at most [`MAX_LINE`] bytes (CRLF or LF ended).
pub(crate) fn read_line_capped<R: BufRead>(reader: &mut R) -> Result<String> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Err(super::imap::ConnectionClosed.into());
        }
        let end = available.iter().position(|&b| b == b'\n');
        let take = end.map_or(available.len(), |i| i + 1);
        if line.len() + take > MAX_LINE {
            bail!("server line too long");
        }
        line.extend_from_slice(&available[..take]);
        reader.consume(take);
        if end.is_some() {
            return Ok(String::from_utf8_lossy(&line).into_owned());
        }
    }
}

/// What the review read from a server: its certificate and, when Windows
/// could build the chain from what the server sent, the certificate of the
/// CA that issued it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probed {
    /// The server's own certificate (DER).
    pub leaf: Vec<u8>,
    /// The issuing CA's certificate (DER): the chain's second element, never
    /// the leaf itself (a self-signed certificate has none).
    pub issuer: Option<Vec<u8>>,
}

/// Read the certificate `host:port` presents, sending no credentials: TCP,
/// then TLS or (STARTTLS) the plain greeting and the STARTTLS exchange, then
/// a handshake that accepts any certificate, then the certificate's DER and
/// its issuer from the chain Windows built, then close. For the user's
/// explicit "Review certificate" only, on a short-lived thread.
///
/// SChannel directly (native-tls hides the chain): its verify callback sees
/// the chain built from the certificates the server sent (with the same
/// cache-only revocation flags as every connection) and accepts it, which is
/// sound only because nothing is sent on this connection.
pub fn probe_certificate(host: &str, port: u16, security: Security, kind: Kind) -> Result<Probed> {
    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .with_context(|| format!("resolving {host}"))?
        .collect();
    if addrs.is_empty() {
        bail!("{host} has no address");
    }
    let tcp = super::imap::connect_any(&super::imap::interleave(addrs))
        .with_context(|| format!("connecting to {host}:{port}"))?;
    tcp.set_read_timeout(Some(PROBE_IO))?;
    tcp.set_write_timeout(Some(PROBE_IO))?;
    let tcp = match (security, kind) {
        (Security::Tls, _) => tcp,
        (Security::StartTls, Kind::Imap) => {
            probe_starttls(tcp, imap_exchange).context("IMAP STARTTLS failed")?
        }
        (Security::StartTls, Kind::Smtp) => {
            probe_starttls(tcp, smtp_exchange).context("SMTP STARTTLS failed")?
        }
    };
    probe_handshake(host, tcp)
}

/// The accept-everything handshake of the probe, keeping the chain.
fn probe_handshake<S: Read + Write>(host: &str, tcp: S) -> Result<Probed> {
    use schannel::schannel_cred::{Direction, SchannelCred};
    use std::sync::{Arc, Mutex};
    let cred = SchannelCred::builder()
        .acquire(Direction::Outbound)
        .context("initializing TLS")?;
    let chain: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
    let seen = Arc::clone(&chain);
    let mut builder = schannel::tls_stream::Builder::new();
    builder
        .domain(host)
        .accept_invalid_hostnames(true)
        .verify_callback(move |res| {
            if let Some(c) = res.chain() {
                let certs = c.certificates().take(8).map(|c| c.to_der().to_vec());
                if let Ok(mut out) = seen.lock() {
                    *out = certs.collect();
                }
            }
            Ok(())
        });
    let stream = builder.connect(cred, tcp).map_err(|e| match e {
        schannel::tls_stream::HandshakeError::Failure(e) => anyhow!("TLS handshake failed: {e}"),
        schannel::tls_stream::HandshakeError::Interrupted(_) => anyhow!("TLS handshake failed"),
    })?;
    let leaf = stream
        .peer_certificate()
        .context("reading the certificate")?
        .to_der()
        .to_vec();
    if leaf.is_empty() || leaf.len() > MAX_PIN {
        bail!("the certificate is too large to pin");
    }
    let chain = chain.lock().map(|c| c.clone()).unwrap_or_default();
    Ok(Probed {
        issuer: issuer_of(&leaf, &chain),
        leaf,
    })
}

/// The CA certificate to offer for `leaf`: the chain's second element, if
/// the chain starts at `leaf` and that element is a different certificate
/// of a usable size. A self-signed leaf, or a chain Windows could not
/// build past the leaf, offers none.
fn issuer_of(leaf: &[u8], chain: &[Vec<u8>]) -> Option<Vec<u8>> {
    match chain {
        [first, ca, ..] if first.as_slice() == leaf && ca.as_slice() != leaf => {
            (!ca.is_empty() && ca.len() <= MAX_PIN).then(|| ca.clone())
        }
        _ => None,
    }
}

/// Run `exchange` over the plain socket; it must leave the socket exactly
/// where the TLS handshake starts. Bytes the server sent after its STARTTLS
/// answer are an attack (they would be read as if they came over TLS).
fn probe_starttls<S: Read + Write>(
    stream: S,
    exchange: fn(&mut BufReader<S>) -> Result<()>,
) -> Result<S> {
    let mut reader = BufReader::new(stream);
    exchange(&mut reader)?;
    if !reader.buffer().is_empty() {
        bail!("unexpected data before TLS");
    }
    Ok(reader.into_inner())
}

fn imap_exchange<S: Read + Write>(reader: &mut BufReader<S>) -> Result<()> {
    super::imap::starttls_exchange(reader)
}

/// The reply code of a line of a (multi-line) SMTP reply, and whether it
/// is the last.
fn smtp_reply_line(line: &str) -> Result<(u16, bool, &str)> {
    let code: u16 = line
        .get(..3)
        .and_then(|c| c.parse().ok())
        .context("not an SMTP reply")?;
    match line.as_bytes().get(3) {
        Some(b'-') => Ok((code, false, &line[4..])),
        Some(b' ') | None | Some(b'\r' | b'\n') => Ok((code, true, line.get(4..).unwrap_or(""))),
        _ => bail!("not an SMTP reply"),
    }
}

fn smtp_reply<S: Read + Write>(reader: &mut BufReader<S>) -> Result<(u16, Vec<String>)> {
    let mut texts = Vec::new();
    for _ in 0..MAX_LINES {
        let line = read_line_capped(reader)?;
        let (code, last, text) = smtp_reply_line(&line)?;
        texts.push(text.trim_end().to_ascii_uppercase());
        if last {
            return Ok((code, texts));
        }
    }
    bail!("too many lines")
}

fn smtp_exchange<S: Read + Write>(reader: &mut BufReader<S>) -> Result<()> {
    if smtp_reply(reader)?.0 != 220 {
        bail!("unexpected greeting");
    }
    // The address literal, as the sending connection greets (no computer name).
    reader.get_mut().write_all(b"EHLO [127.0.0.1]\r\n")?;
    let (code, caps) = smtp_reply(reader)?;
    if code != 250 || !caps.iter().any(|c| c == "STARTTLS") {
        return Err(super::imap::StartTlsRefused.into());
    }
    reader.get_mut().write_all(b"STARTTLS\r\n")?;
    if smtp_reply(reader)?.0 != 220 {
        return Err(super::imap::StartTlsRefused.into());
    }
    Ok(())
}

// ---- fingerprint -------------------------------------------------------

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// SHA-256 of `data` (FIPS 180-4). Used to show a fingerprint; about forty
/// lines instead of a dependency.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for (a, b) in h.iter_mut().zip(v) {
            *a = a.wrapping_add(b);
        }
    }
    let mut out = [0u8; 32];
    for (chunk, word) in out.chunks_exact_mut(4).zip(h) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// The SHA-256 of a certificate as `AB:CD:...` (64 hex digits, 32 groups).
pub fn fingerprint_sha256(der: &[u8]) -> String {
    let digest = sha256(der);
    let mut out = String::with_capacity(95);
    for (i, b) in digest.iter().enumerate() {
        if i > 0 {
            out.push(':');
        }
        out.push_str(&format!("{b:02X}"));
    }
    out
}

/// The start of the SHA-256 of a certificate's key (SubjectPublicKeyInfo),
/// for a status line; empty if the key cannot be read.
pub fn key_fingerprint_short(der: &[u8]) -> String {
    super::x509::spki_sha256(der)
        .map(|d| {
            d.iter()
                .take(8)
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(":")
        })
        .unwrap_or_default()
}

/// The SHA-256 of a certificate's key as `AB:CD:...`.
pub fn key_fingerprint_sha256(der: &[u8]) -> String {
    super::x509::spki_sha256(der)
        .map(|d| {
            d.iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(":")
        })
        .unwrap_or_default()
}

/// The start of a fingerprint, for a status line (the prompt shows all of it).
pub fn fingerprint_short(der: &[u8]) -> String {
    fingerprint_sha256(der)[..23].to_string()
}

// ---- display-only certificate details ---------------------------------

/// What the prompt shows about a certificate. Read minimally from the DER
/// for display only (no trust decision uses it); a certificate this reader
/// cannot follow shows its fingerprint alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertInfo {
    pub subject: String,
    /// DNS names and IP addresses of the subject alternative names.
    pub names: Vec<String>,
    pub issuer: String,
    pub self_signed: bool,
    /// `YYYY-MM-DD`.
    pub not_before: String,
    pub not_after: String,
}

use super::x509::{Der, items, take};

/// Text from a DER string, without control characters, at most 80 chars.
fn der_string(item: &Der) -> String {
    clean(&String::from_utf8_lossy(item.body))
}

fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(80)
        .collect::<String>()
        .trim()
        .to_string()
}

/// A name's common name, or else its organization.
fn name_text(name: &Der) -> Option<String> {
    let mut cn = None;
    for set in items(name.body)? {
        for pair in items(set.body)? {
            let parts = items(pair.body)?;
            let (oid, value) = (parts.first()?, parts.get(1)?);
            if oid.body == [0x55, 0x04, 0x03] {
                cn = cn.or(Some(der_string(value)));
            }
        }
    }
    cn.filter(|s| !s.is_empty())
}

fn der_date(item: &Der) -> Option<String> {
    let text = std::str::from_utf8(item.body).ok()?;
    let (year, rest) = match item.tag {
        0x17 => {
            let yy: u32 = text.get(..2)?.parse().ok()?;
            (if yy >= 50 { 1900 + yy } else { 2000 + yy }, text.get(2..)?)
        }
        0x18 => (text.get(..4)?.parse().ok()?, text.get(4..)?),
        _ => return None,
    };
    let (month, day): (u32, u32) = (rest.get(..2)?.parse().ok()?, rest.get(2..4)?.parse().ok()?);
    ((1..=12).contains(&month) && (1..=31).contains(&day))
        .then(|| format!("{year:04}-{month:02}-{day:02}"))
}

/// The subject alternative names in an extensions sequence.
fn alt_names(extensions: &Der) -> Vec<String> {
    let mut out = Vec::new();
    let Some(list) = items(extensions.body) else {
        return out;
    };
    for ext in list {
        let Some(parts) = items(ext.body) else {
            continue;
        };
        let (Some(oid), Some(value)) = (parts.first(), parts.last()) else {
            continue;
        };
        if oid.body != [0x55, 0x1d, 0x11] || value.tag != 0x04 {
            continue;
        }
        let Some((names, _)) = take(value.body) else {
            continue;
        };
        for name in items(names.body).unwrap_or_default() {
            match (name.tag, name.body.len()) {
                (0x82, _) => out.push(clean(&String::from_utf8_lossy(name.body))),
                (0x87, 4) => out.push(
                    Ipv4Addr::new(name.body[0], name.body[1], name.body[2], name.body[3])
                        .to_string(),
                ),
                _ => {}
            }
        }
    }
    out.retain(|n| !n.is_empty());
    out
}

/// Details of the certificate `der` for display, if it can be read.
pub fn describe(der: &[u8]) -> Option<CertInfo> {
    let (cert, _) = take(der)?;
    let (tbs, _) = take(cert.body)?;
    let mut fields = items(tbs.body)?.into_iter();
    let mut next = fields.next()?;
    if next.tag == 0xa0 {
        next = fields.next()?; // the version; `next` was the serial
    }
    let _serial = next;
    let _algorithm = fields.next()?;
    let issuer = fields.next()?;
    let validity = fields.next()?;
    let subject = fields.next()?;
    let _key = fields.next()?;
    let dates = items(validity.body)?;
    let names = fields
        .find(|f| f.tag == 0xa3)
        .and_then(|f| take(f.body).map(|(seq, _)| alt_names(&seq)))
        .unwrap_or_default();
    Some(CertInfo {
        subject: name_text(&subject).unwrap_or_default(),
        names: names.into_iter().take(8).collect(),
        issuer: name_text(&issuer).unwrap_or_default(),
        self_signed: issuer.whole == subject.whole,
        not_before: der_date(dates.first()?)?,
        not_after: der_date(dates.get(1)?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::fake_tls::{
        CERT, KEY, Mode, OTHER_CERT, OTHER_KEY, Proto, TlsFake, acceptor, der_of,
    };
    use crate::mail::imap::Imap;
    use std::io::{BufRead, BufReader};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::thread;

    fn imap_server(addr: SocketAddr, security: Security) -> crate::providers::Server {
        crate::providers::Server {
            host: "localhost".into(),
            port: addr.port(),
            security,
        }
    }

    // ---- the hash and the fingerprint

    #[test]
    fn sha256_matches_the_nist_vectors() {
        let hex = |d: [u8; 32]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(
            hex(sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // Several blocks.
        assert_eq!(
            hex(sha256(&[b'a'; 1000])),
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
        );
    }

    #[test]
    fn fingerprint_is_the_grouped_hash_of_the_certificate() {
        let der = der_of(CERT);
        let fp = fingerprint_sha256(&der);
        assert_eq!(fp.len(), 95);
        assert!(fp.split(':').all(|g| {
            g.len() == 2
                && g.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_lowercase())
        }));
        // Computed offline: `openssl x509 -in test-cert.pem -noout -fingerprint -sha256`.
        assert_eq!(fp, FIXTURE_FINGERPRINT);
        assert_eq!(
            fingerprint_short(&der),
            FIXTURE_FINGERPRINT
                .split(':')
                .take(8)
                .collect::<Vec<_>>()
                .join(":")
        );
    }

    const FIXTURE_FINGERPRINT: &str = "F7:AE:3F:F2:B3:50:84:AB:53:1A:A9:1D:FC:12:AC:BB:67:A7:85:F0:69:9D:12:AB:77:76:7F:2C:6A:77:1B:9A";

    // ---- states

    #[test]
    fn a_corrupt_or_oversized_pin_is_no_pin() {
        let der = der_of(CERT);
        assert_eq!(state(None), PinState::None);
        assert_eq!(
            state(Some(&der)),
            PinState::Pinned(Pin {
                trust: Trust::Certificate,
                der: &der
            })
        );
        assert_eq!(state(Some(&[])), PinState::Damaged);
        assert_eq!(state(Some(&[1, 2, 3])), PinState::Damaged);
        let mut big = der.clone();
        big.resize(MAX_PIN + 1, 0);
        assert_eq!(state(Some(&big)), PinState::Damaged);
        let mut cut = der.clone();
        cut.truncate(100);
        assert_eq!(state(Some(&cut)), PinState::Damaged);
        assert_eq!(usable(Some(&[1, 2, 3])), None);
        assert_eq!(usable(Some(&der)).map(|p| p.der), Some(&der[..]));
    }

    #[test]
    fn a_damaged_pin_trusts_nothing() {
        // The connector is the default one: the self-signed fake fails.
        let fake = TlsFake::start(Proto::Imap, Mode::Implicit, CERT, KEY);
        let server = imap_server(fake.addr, Security::Tls);
        let err = Imap::connect(&server, Some(&[1, 2, 3]), "alice", "pw")
            .err()
            .expect("a damaged pin trusts nothing");
        assert!(
            err.chain().any(|c| c.is::<crate::mail::imap::TlsFailed>()),
            "{err:#}"
        );
        assert!(fake.finish().is_empty());
    }

    // ---- display details

    #[test]
    fn describes_the_fixture_certificate() {
        let info = describe(&der_of(CERT)).expect("readable");
        assert_eq!(info.subject, "localhost");
        assert_eq!(info.issuer, "localhost");
        assert!(info.self_signed);
        assert_eq!(info.names, ["127.0.0.1", "localhost"]);
        assert_eq!(info.not_before.len(), 10);
        assert!(info.not_after > info.not_before);
    }

    #[test]
    fn describing_unreadable_input_gives_nothing_and_never_panics() {
        assert_eq!(describe(&[]), None);
        assert_eq!(describe(&[0x30, 0x84, 0xff, 0xff, 0xff, 0xff]), None);
        let der = der_of(CERT);
        for n in (0..der.len()).step_by(7) {
            let _ = describe(&der[..n]);
        }
        let mut flipped = der.clone();
        for i in (0..flipped.len()).step_by(3) {
            flipped[i] ^= 0x5a;
            let _ = describe(&flipped);
            flipped[i] ^= 0x5a;
        }
    }

    // ---- the connector (the spike)

    fn serve_once(cert: &'static [u8], key: &'static [u8]) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let acceptor = acceptor(cert, key);
        thread::spawn(move || {
            if let Ok((s, _)) = listener.accept()
                && let Ok(mut t) = acceptor.accept(s)
            {
                t.write_all(b"hi\r\n").ok();
                let mut b = [0u8; 4];
                let _ = t.read(&mut b);
            }
        });
        port
    }

    fn handshake(c: &TlsConnector, host: &str, port: u16) -> Result<Vec<u8>, String> {
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut t = c.connect(host, tcp).map_err(|e| e.to_string())?;
        let mut b = [0u8; 4];
        t.read_exact(&mut b).map_err(|e| e.to_string())?;
        Ok(t.peer_certificate().unwrap().unwrap().to_der().unwrap())
    }

    #[test]
    fn without_a_pin_the_self_signed_certificate_is_refused() {
        let port = serve_once(CERT, KEY);
        assert!(handshake(&connector(None).unwrap(), "localhost", port).is_err());
    }

    #[test]
    fn the_pinned_certificate_is_trusted_by_name_or_address() {
        let der = der_of(CERT);
        for host in ["localhost", "127.0.0.1"] {
            let port = serve_once(CERT, KEY);
            let got = handshake(&connector(Some(&der)).unwrap(), host, port);
            assert_eq!(got.as_deref(), Ok(&der[..]), "{host}");
        }
    }

    #[test]
    fn another_certificate_for_the_same_names_is_refused() {
        // The other certificate has the same subject and names.
        let pinned = der_of(CERT);
        for host in ["localhost", "127.0.0.1"] {
            let port = serve_once(OTHER_CERT, OTHER_KEY);
            assert!(
                handshake(&connector(Some(&pinned)).unwrap(), host, port).is_err(),
                "{host}"
            );
        }
    }

    // ---- the probe

    #[test]
    fn the_probe_returns_the_certificate_and_sends_nothing() {
        let der = der_of(CERT);
        for (proto, mode, security) in [
            (Proto::Imap, Mode::Implicit, Security::Tls),
            (Proto::Imap, Mode::StartTls, Security::StartTls),
            (Proto::Smtp, Mode::Implicit, Security::Tls),
            (Proto::Smtp, Mode::StartTls, Security::StartTls),
        ] {
            let fake = TlsFake::start(proto, mode, CERT, KEY);
            let kind = if proto == Proto::Imap {
                Kind::Imap
            } else {
                Kind::Smtp
            };
            let got = probe_certificate("localhost", fake.addr.port(), security, kind).unwrap();
            assert_eq!(got.leaf, der);
            // A self-signed certificate has no issuer to offer.
            assert_eq!(got.issuer, None);
            let seen = fake.finish();
            let upper: Vec<String> = seen.iter().map(|l| l.to_ascii_uppercase()).collect();
            assert!(
                !upper
                    .iter()
                    .any(|l| l.contains("LOGIN") || l.contains("AUTH")),
                "{seen:?}"
            );
            if mode == Mode::StartTls {
                assert!(upper.iter().any(|l| l.contains("STARTTLS")), "{seen:?}");
            } else {
                assert!(seen.is_empty(), "{seen:?}");
            }
        }
    }

    #[test]
    fn the_probe_refuses_a_server_without_starttls() {
        for proto in [Proto::Imap, Proto::Smtp] {
            let fake = TlsFake::start(proto, Mode::NoStartTls, CERT, KEY);
            let kind = if proto == Proto::Imap {
                Kind::Imap
            } else {
                Kind::Smtp
            };
            let err = probe_certificate("localhost", fake.addr.port(), Security::StartTls, kind)
                .unwrap_err();
            assert!(
                err.chain()
                    .any(|c| c.is::<crate::mail::imap::StartTlsRefused>()),
                "{err:#}"
            );
            let seen = fake.finish();
            assert!(
                !seen.iter().any(|l| l.to_ascii_uppercase().contains("LOGIN")
                    || l.to_ascii_uppercase().starts_with("AUTH"))
            );
        }
    }

    #[test]
    fn the_probe_refuses_bytes_sent_before_the_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.write_all(b"* OK hi\r\n").unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            s.write_all(b"* CAPABILITY IMAP4rev1 STARTTLS\r\na0 OK done\r\n")
                .unwrap();
            line.clear();
            r.read_line(&mut line).unwrap();
            s.write_all(b"a1 OK go\r\n* OK injected\r\n").unwrap();
            thread::sleep(Duration::from_millis(300));
        });
        let err = probe_certificate("127.0.0.1", port, Security::StartTls, Kind::Imap).unwrap_err();
        assert!(
            format!("{err:#}").contains("unexpected data before TLS"),
            "{err:#}"
        );
        server.join().unwrap();
    }

    #[test]
    fn the_probe_caps_a_line() {
        let long = vec![b'x'; MAX_LINE + 10];
        let mut reader = BufReader::new(&long[..]);
        assert!(read_line_capped(&mut reader).is_err());
        let mut ok = BufReader::new(&b"* OK hi\r\nnext\r\n"[..]);
        assert_eq!(read_line_capped(&mut ok).unwrap(), "* OK hi\r\n");
    }

    // ---- sign-in with and without a pin

    #[test]
    fn unpinned_self_signed_is_refused_before_any_credentials() {
        for mode in [Mode::Implicit, Mode::StartTls] {
            let fake = TlsFake::start(Proto::Imap, mode, CERT, KEY);
            let security = if mode == Mode::Implicit {
                Security::Tls
            } else {
                Security::StartTls
            };
            let err = Imap::connect(&imap_server(fake.addr, security), None, "alice", "pw")
                .err()
                .expect("untrusted");
            assert!(
                err.chain().any(|c| c.is::<crate::mail::imap::TlsFailed>()),
                "{err:#}"
            );
            let seen = fake.finish();
            assert!(!seen.iter().any(|l| l.contains("LOGIN")), "{seen:?}");
        }
    }

    #[test]
    fn a_pinned_imap_server_signs_in_by_name_and_by_address() {
        let der = der_of(CERT);
        for (host, mode) in [
            ("localhost", Mode::Implicit),
            ("127.0.0.1", Mode::Implicit),
            ("localhost", Mode::StartTls),
        ] {
            let fake = TlsFake::start(Proto::Imap, mode, CERT, KEY);
            let mut server = imap_server(
                fake.addr,
                if mode == Mode::Implicit {
                    Security::Tls
                } else {
                    Security::StartTls
                },
            );
            server.host = host.into();
            let imap = Imap::connect(&server, Some(&der), "alice", "pw").expect("pinned sign-in");
            imap.logout();
            let seen = fake.finish();
            assert!(seen.iter().any(|l| l.contains("LOGIN")), "{host} {seen:?}");
        }
    }

    #[test]
    fn a_swapped_imap_certificate_is_refused_before_any_credentials() {
        let pinned = der_of(CERT);
        for mode in [Mode::Implicit, Mode::StartTls] {
            let fake = TlsFake::start(Proto::Imap, mode, OTHER_CERT, OTHER_KEY);
            let security = if mode == Mode::Implicit {
                Security::Tls
            } else {
                Security::StartTls
            };
            let err = Imap::connect(
                &imap_server(fake.addr, security),
                Some(&pinned),
                "alice",
                "pw",
            )
            .err()
            .expect("swapped certificate");
            assert!(err.chain().any(|c| c.is::<PinMismatch>()), "{err:#}");
            let seen = fake.finish();
            assert!(!seen.iter().any(|l| l.contains("LOGIN")), "{seen:?}");
        }
    }

    #[test]
    fn check_peer_compares_bytes() {
        // The connector alone would accept this (pinned = presented); a
        // different pin is a mismatch even when the handshake passed.
        let fake = TlsFake::start(Proto::Imap, Mode::Implicit, CERT, KEY);
        let tcp = TcpStream::connect(fake.addr).unwrap();
        let tls = TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true)
            .build()
            .unwrap();
        let stream = tls.connect("localhost", tcp).unwrap();
        let ours = der_of(CERT);
        let theirs = der_of(OTHER_CERT);
        assert!(check_peer(&stream, Some(&ours), "localhost").is_ok());
        assert!(check_peer(&stream, None, "localhost").is_ok());
        let err = check_peer(&stream, Some(&theirs), "localhost").unwrap_err();
        assert!(err.is::<PinMismatch>());
        // A damaged pin is no pin.
        assert!(check_peer(&stream, Some(&[1, 2, 3]), "localhost").is_ok());
        drop(stream);
        fake.finish();
    }

    // ---- SMTP

    use crate::mail::errors::{Class, Special, Step as Check, StepError};
    use crate::mail::smtp::verify_smtp;

    fn smtp_check(
        mode: Mode,
        served: (&'static [u8], &'static [u8]),
        pin: Option<&[u8]>,
    ) -> (Result<(), StepError>, Vec<String>) {
        let fake = TlsFake::start(Proto::Smtp, mode, served.0, served.1);
        let security = if mode == Mode::Implicit {
            Security::Tls
        } else {
            Security::StartTls
        };
        let server = imap_server(fake.addr, security);
        let result = verify_smtp(&server, pin, "relay", "pw", true, &mut |_| {});
        (result, fake.finish())
    }

    fn sent_auth(seen: &[String]) -> bool {
        seen.iter()
            .any(|l| l.to_ascii_uppercase().starts_with("AUTH"))
    }

    const RENEWED: &[u8] = include_bytes!("../../tests/fixtures/tls/renewed-cert.pem");
    const CA: &[u8] = include_bytes!("../../tests/fixtures/tls/ca-cert.pem");
    const CA_LEAF: &[u8] = include_bytes!("../../tests/fixtures/tls/ca-leaf-cert.pem");
    const CA_LEAF2: &[u8] = include_bytes!("../../tests/fixtures/tls/ca-leaf2-cert.pem");

    #[test]
    fn a_key_pin_passes_a_renewed_certificate_and_refuses_another_key() {
        let pin = encode(Trust::Key, &der_of(CERT));
        assert_eq!(usable(Some(&pin)).map(|p| p.trust), Some(Trust::Key));
        for mode in [Mode::Implicit, Mode::StartTls] {
            for served in [CERT, RENEWED] {
                let (result, seen) = smtp_check(mode, (served, KEY), Some(&pin));
                result.unwrap_or_else(|e| panic!("{:#}", e.source));
                assert!(sent_auth(&seen), "{seen:?}");
            }
            let (result, seen) = smtp_check(mode, (OTHER_CERT, OTHER_KEY), Some(&pin));
            let e = result.unwrap_err();
            assert_eq!(e.special, Special::CertChanged, "{:#}", e.source);
            assert!(!sent_auth(&seen), "{seen:?}");
        }
    }

    #[test]
    fn a_ca_pin_passes_what_the_ca_issued_and_refuses_the_rest() {
        let pin = encode(Trust::Ca, &der_of(CA));
        assert_eq!(usable(Some(&pin)).map(|p| p.trust), Some(Trust::Ca));
        for mode in [Mode::Implicit, Mode::StartTls] {
            // Two certificates the CA issued, with different keys.
            for (cert, key) in [(CA_LEAF, OTHER_KEY), (CA_LEAF2, KEY)] {
                let (result, seen) = smtp_check(mode, (cert, key), Some(&pin));
                result.unwrap_or_else(|e| panic!("{:#}", e.source));
                assert!(sent_auth(&seen), "{seen:?}");
            }
            // A self-signed certificate for the same names is refused.
            let (result, seen) = smtp_check(mode, (CERT, KEY), Some(&pin));
            let e = result.unwrap_err();
            assert_eq!(e.special, Special::CertChanged, "{:#}", e.source);
            assert!(!sent_auth(&seen), "{seen:?}");
        }
    }

    #[test]
    fn an_exact_pin_refuses_a_renewed_certificate() {
        let pin = der_of(CERT);
        let (result, seen) = smtp_check(Mode::Implicit, (RENEWED, KEY), Some(&pin));
        assert_eq!(result.unwrap_err().special, Special::CertChanged);
        assert!(!sent_auth(&seen), "{seen:?}");
    }

    #[test]
    fn a_key_or_ca_pin_on_imap_is_checked_before_any_credentials() {
        let key = encode(Trust::Key, &der_of(CERT));
        let ca = encode(Trust::Ca, &der_of(CA));
        for (pin, served, ok) in [
            (&key, (RENEWED, KEY), true),
            (&key, (OTHER_CERT, OTHER_KEY), false),
            (&ca, (CA_LEAF, OTHER_KEY), true),
            (&ca, (CERT, KEY), false),
        ] {
            let fake = TlsFake::start(Proto::Imap, Mode::Implicit, served.0, served.1);
            let server = imap_server(fake.addr, Security::Tls);
            let result = crate::mail::imap::Imap::connect(&server, Some(pin), "me", "pw");
            let seen = fake.finish();
            assert_eq!(result.is_ok(), ok, "{seen:?}");
            assert_eq!(seen.iter().any(|l| l.contains("LOGIN")), ok, "{seen:?}");
        }
    }

    /// One TLS handshake on `127.0.0.1` presenting `leaf` (with `key`) and,
    /// in the identity's store, the further certificates `chain` (what a
    /// server configured with its CA sends), then close.
    fn chain_server(
        leaf: &'static [u8],
        key: &'static [u8],
        chain: &'static [&'static [u8]],
    ) -> u16 {
        use schannel::cert_context::{CertContext, KeySpec};
        use schannel::cert_store::{CertAdd, Memory};
        use schannel::crypt_prov::{AcquireOptions, ProviderType};
        use schannel::schannel_cred::{Direction, SchannelCred};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let mut store = Memory::new().unwrap().into_store();
            let cert = CertContext::from_pem(std::str::from_utf8(leaf).unwrap()).unwrap();
            let name = format!("komorebi-chain-test-{port}");
            let mut options = AcquireOptions::new();
            options.container(&name);
            let mut prov = match options.acquire(ProviderType::rsa_full()) {
                Ok(p) => p,
                Err(_) => options
                    .new_keyset(true)
                    .acquire(ProviderType::rsa_full())
                    .unwrap(),
            };
            prov.import().import_pkcs8_pem(key).unwrap();
            cert.set_key_prov_info()
                .container(&name)
                .type_(ProviderType::rsa_full())
                .keep_open(true)
                .key_spec(KeySpec::key_exchange())
                .set()
                .unwrap();
            let context = store.add_cert(&cert, CertAdd::Always).unwrap();
            for pem in chain {
                let c = CertContext::from_pem(std::str::from_utf8(pem).unwrap()).unwrap();
                store.add_cert(&c, CertAdd::Always).unwrap();
            }
            let cred = SchannelCred::builder()
                .cert(context)
                .acquire(Direction::Inbound)
                .unwrap();
            let (tcp, _) = listener.accept().unwrap();
            let _ = schannel::tls_stream::Builder::new().accept(cred, tcp);
        });
        port
    }

    #[test]
    fn the_review_offers_no_ca_that_is_not_in_the_chain() {
        // SChannel as a server sends only a chain that ends at a root its
        // machine trusts, so a fake cannot present the private test CA and
        // the offered path is covered by `issuer_of` below, not end to end.
        let port = chain_server(CA_LEAF, OTHER_KEY, &[]);
        let got = probe_certificate("127.0.0.1", port, Security::Tls, Kind::Imap).unwrap();
        assert_eq!(got.leaf, der_of(CA_LEAF));
        assert_eq!(got.issuer, None);
    }

    #[test]
    fn the_issuer_is_never_the_leaf_and_never_oversized() {
        let leaf = der_of(CA_LEAF);
        let ca = der_of(CA);
        assert_eq!(issuer_of(&leaf, std::slice::from_ref(&leaf)), None);
        assert_eq!(issuer_of(&leaf, &[leaf.clone(), leaf.clone()]), None);
        assert_eq!(issuer_of(&leaf, &[ca.clone(), leaf.clone()]), None);
        assert_eq!(
            issuer_of(&leaf, &[leaf.clone(), vec![0; MAX_PIN + 1]]),
            None
        );
        assert_eq!(issuer_of(&leaf, &[leaf.clone(), ca.clone()]), Some(ca));
        assert_eq!(issuer_of(&leaf, &[]), None);
    }

    #[test]
    fn tagged_pins_round_trip_and_unknown_tags_are_damaged() {
        let der = der_of(CERT);
        assert_eq!(encode(Trust::Certificate, &der), der);
        for trust in [Trust::Key, Trust::Ca] {
            let blob = encode(trust, &der);
            assert_eq!(usable(Some(&blob)), Some(Pin { trust, der: &der }));
        }
        let mut odd = b"KPIN".to_vec();
        odd.push(9);
        odd.extend_from_slice(&der);
        assert_eq!(state(Some(&odd)), PinState::Damaged);
        assert_eq!(state(Some(b"KPIN")), PinState::Damaged);
    }

    #[test]
    fn a_relay_without_sign_in_gets_no_auth_and_still_needs_tls() {
        let fake = TlsFake::start(Proto::Smtp, Mode::StartTls, CERT, KEY);
        let server = imap_server(fake.addr, Security::StartTls);
        let pin = der_of(CERT);
        verify_smtp(&server, Some(&pin), "", "", false, &mut |_| {})
            .unwrap_or_else(|e| panic!("{:#}", e.source));
        let seen = fake.finish();
        assert!(!sent_auth(&seen), "{seen:?}");
        assert!(seen.iter().any(|l| l.starts_with("STARTTLS")), "{seen:?}");
        assert!(seen.iter().any(|l| l.starts_with("NOOP")), "{seen:?}");
        // Without STARTTLS on offer nothing goes on in the clear.
        let fake = TlsFake::start(Proto::Smtp, Mode::NoStartTls, CERT, KEY);
        let server = imap_server(fake.addr, Security::StartTls);
        assert!(verify_smtp(&server, Some(&pin), "", "", false, &mut |_| {}).is_err());
        let seen = fake.finish();
        assert!(
            !seen
                .iter()
                .any(|l| l.starts_with("MAIL") || l.starts_with("NOOP"))
        );
    }

    #[test]
    fn an_account_without_smtp_sign_in_sends_over_tls_without_auth_or_password() {
        use lettre::address::{Address, Envelope};
        let me: Address = "me@example.com".parse().unwrap();
        let envelope = Envelope::new(Some(me.clone()), vec![me]).unwrap();
        let raw = b"From: me@example.com\r\nTo: me@example.com\r\nSubject: x\r\n\r\nhi\r\n";
        for mode in [Mode::Implicit, Mode::StartTls] {
            let fake = TlsFake::start(Proto::Smtp, mode, CERT, KEY);
            let security = if mode == Mode::Implicit {
                Security::Tls
            } else {
                Security::StartTls
            };
            let server = imap_server(fake.addr, security);
            let account = crate::db::Account {
                // No password is saved for this id: reading one would fail.
                id: -4242,
                email: "me@example.com".into(),
                name: String::new(),
                imap: server.clone(),
                smtp: server,
                save_sent: false,
                signature: String::new(),
                enabled: true,
                imap_user: String::new(),
                smtp_user: String::new(),
                imap_pin: None,
                smtp_pin: Some(der_of(CERT)),
                smtp_no_auth: true,
            };
            super::super::smtp_account(&account, &envelope, raw)
                .unwrap_or_else(|e| panic!("{e:#}"));
            let seen = fake.finish();
            assert!(!sent_auth(&seen), "{seen:?}");
            assert!(seen.iter().any(|l| l == "DATA"), "{seen:?}");
            if mode == Mode::StartTls {
                assert!(seen.iter().any(|l| l.starts_with("STARTTLS")), "{seen:?}");
            }
        }
        // Without STARTTLS on offer nothing is sent in the clear.
        let fake = TlsFake::start(Proto::Smtp, Mode::NoStartTls, CERT, KEY);
        let server = imap_server(fake.addr, Security::StartTls);
        let account = crate::db::Account {
            id: -4242,
            email: "me@example.com".into(),
            name: String::new(),
            imap: server.clone(),
            smtp: server,
            save_sent: false,
            signature: String::new(),
            enabled: true,
            imap_user: String::new(),
            smtp_user: String::new(),
            imap_pin: None,
            smtp_pin: Some(der_of(CERT)),
            smtp_no_auth: true,
        };
        assert!(super::super::smtp_account(&account, &envelope, raw).is_err());
        let seen = fake.finish();
        assert!(
            !seen.iter().any(|l| l.starts_with("MAIL") || l == "DATA"),
            "{seen:?}"
        );
    }

    #[test]
    fn unpinned_self_signed_smtp_is_refused_before_any_credentials() {
        for mode in [Mode::Implicit, Mode::StartTls] {
            let (result, seen) = smtp_check(mode, (CERT, KEY), None);
            let e = result.unwrap_err();
            assert_eq!(e.class, Class::TlsCert, "{:#}", e.source);
            assert_eq!(e.special, Special::None);
            assert!(!sent_auth(&seen), "{seen:?}");
        }
    }

    #[test]
    fn a_pinned_smtp_server_signs_in_and_greets_without_the_computer_name() {
        let der = der_of(CERT);
        for mode in [Mode::Implicit, Mode::StartTls] {
            let (result, seen) = smtp_check(mode, (CERT, KEY), Some(&der));
            result.unwrap_or_else(|e| panic!("{:#}", e.source));
            assert!(sent_auth(&seen), "{seen:?}");
            let ehlo: Vec<_> = seen.iter().filter(|l| l.starts_with("EHLO")).collect();
            assert!(
                !ehlo.is_empty() && ehlo.iter().all(|l| *l == "EHLO [127.0.0.1]"),
                "{seen:?}"
            );
        }
    }

    #[test]
    fn a_swapped_smtp_certificate_is_refused_before_any_credentials() {
        let pinned = der_of(CERT);
        for mode in [Mode::Implicit, Mode::StartTls] {
            let (result, seen) = smtp_check(mode, (OTHER_CERT, OTHER_KEY), Some(&pinned));
            let e = result.unwrap_err();
            assert_eq!(
                (e.step, e.class, e.special),
                (Check::SmtpConnect, Class::TlsCert, Special::CertChanged)
            );
            assert_eq!(
                e.say(),
                crate::mail::errors::Say::CertChanged {
                    host: "localhost".into()
                }
            );
            assert!(!sent_auth(&seen), "{seen:?}");
            // The full chain keeps the cause for the diagnostics log.
            assert!(e.source.chain().any(|c| c.is::<PinMismatch>()));
        }
    }

    #[test]
    fn smtp_without_starttls_gets_no_credentials_even_with_a_pin() {
        let der = der_of(CERT);
        for pin in [None, Some(&der[..])] {
            let (result, seen) = smtp_check(Mode::NoStartTls, (CERT, KEY), pin);
            let e = result.unwrap_err();
            assert_eq!(e.special, Special::NoStartTls, "{:#}", e.source);
            assert!(!sent_auth(&seen), "{seen:?}");
        }
    }

    #[test]
    fn imap_without_starttls_gets_no_credentials_even_with_a_pin() {
        let der = der_of(CERT);
        for pin in [None, Some(&der[..])] {
            let fake = TlsFake::start(Proto::Imap, Mode::NoStartTls, CERT, KEY);
            let err = Imap::connect(
                &imap_server(fake.addr, Security::StartTls),
                pin,
                "alice",
                "pw",
            )
            .err()
            .expect("refused");
            assert!(
                err.chain()
                    .any(|c| c.is::<crate::mail::imap::StartTlsRefused>()),
                "{err:#}"
            );
            assert!(!fake.finish().iter().any(|l| l.contains("LOGIN")));
        }
    }

    #[test]
    fn the_form_check_names_a_changed_certificate_and_an_untrusted_one() {
        use crate::mail::verify_account;
        let pinned = der_of(CERT);
        // IMAP: swapped certificate, pinned.
        let imap = TlsFake::start(Proto::Imap, Mode::Implicit, OTHER_CERT, OTHER_KEY);
        let smtp = TlsFake::start(Proto::Smtp, Mode::Implicit, CERT, KEY);
        let (i, s) = (
            imap_server(imap.addr, Security::Tls),
            imap_server(smtp.addr, Security::Tls),
        );
        let mut steps = Vec::new();
        let e = verify_account(&i, &s, (Some(&pinned), None), ("a", "a"), "pw", &mut |c| {
            steps.push(c)
        })
        .unwrap_err();
        assert_eq!(
            (e.step, e.class, e.special),
            (Check::Secure, Class::TlsCert, Special::CertChanged)
        );
        assert_eq!(steps, [Check::Resolve, Check::Connect]);
        drop((imap, smtp));
        // IMAP: untrusted and not pinned is the ordinary certificate failure.
        let imap = TlsFake::start(Proto::Imap, Mode::Implicit, CERT, KEY);
        let i = imap_server(imap.addr, Security::Tls);
        let e = verify_account(&i, &s, (None, None), ("a", "a"), "pw", &mut |_| {}).unwrap_err();
        assert_eq!(
            (e.step, e.class, e.special),
            (Check::Secure, Class::TlsCert, Special::None)
        );
        drop(imap);
    }

    #[test]
    fn a_background_failure_with_a_pin_says_the_certificate_changed() {
        use crate::mail::errors::Problem;
        let pinned = der_of(CERT);
        let fake = TlsFake::start(Proto::Imap, Mode::Implicit, OTHER_CERT, OTHER_KEY);
        let err = Imap::connect(
            &imap_server(fake.addr, Security::Tls),
            Some(&pinned),
            "alice",
            "pw",
        )
        .err()
        .unwrap();
        let problem = Problem::of(&err);
        assert_eq!(problem.class, Class::TlsCert);
        assert_eq!(
            problem.say(),
            Some(crate::mail::errors::Say::CertChanged {
                host: "localhost".into()
            })
        );
        let plain = Problem::of(&anyhow::Error::new(crate::mail::imap::TlsFailed(
            "h".into(),
        )));
        assert_eq!(
            plain.say(),
            Some(crate::mail::errors::Say::Class(Class::TlsCert))
        );
        assert_ne!(plain.key(), problem.key());
        drop(fake);
    }

    #[test]
    fn a_rejected_password_is_said_as_rejected_with_its_login() {
        use crate::mail::errors::Say;
        use crate::mail::verify_account;
        // A trusted (pinned) server that refuses the password: the check
        // stops at the IMAP sign-in with the sentence the account form
        // turns into Set username, and the SMTP server is never reached.
        let imap = TlsFake::start_with(Proto::Imap, Mode::Implicit, CERT, KEY, true);
        let i = imap_server(imap.addr, Security::Tls);
        let s = imap_server(imap.addr, Security::Tls);
        let pinned = der_of(CERT);
        let mut seen = Vec::new();
        let e = verify_account(
            &i,
            &s,
            (Some(&pinned), Some(&pinned)),
            ("alice", "alice"),
            "wrong",
            &mut |c| seen.push(c),
        )
        .unwrap_err();
        assert_eq!((e.step, e.class), (Check::ImapSignIn, Class::Auth));
        assert_eq!(
            e.say(),
            Say::Rejected {
                host: "localhost".into(),
                login: "alice".into()
            }
        );
        assert!(!seen.contains(&Check::SmtpConnect));
        assert!(imap.finish().iter().any(|l| l.contains("LOGIN")));
    }

    #[test]
    fn a_self_signed_account_is_added_only_after_the_certificate_is_confirmed() {
        use crate::db::{Db, NewAccount};
        use crate::mail::verify_account;
        let temp =
            std::env::temp_dir().join(format!("komorebi-pin-e2e-{}.sqlite3", std::process::id()));
        for suffix in ["", "-wal", "-shm"] {
            std::fs::remove_file(format!("{}{suffix}", temp.display())).ok();
        }
        let fakes = || {
            (
                TlsFake::start(Proto::Imap, Mode::Implicit, CERT, KEY),
                TlsFake::start(Proto::Smtp, Mode::StartTls, CERT, KEY),
            )
        };

        // 1. As typed, Windows does not trust it: the check stops at TLS and
        //    nothing was sent.
        let (imap, smtp) = fakes();
        let (i, s) = (
            imap_server(imap.addr, Security::Tls),
            imap_server(smtp.addr, Security::StartTls),
        );
        let mut seen = Vec::new();
        let e = verify_account(&i, &s, (None, None), ("alice", "alice"), "pw", &mut |c| {
            seen.push(c)
        })
        .unwrap_err();
        assert_eq!(
            (e.step, e.class, e.special),
            (Check::Secure, Class::TlsCert, Special::None)
        );
        assert!(!imap.finish().iter().any(|l| l.contains("LOGIN")));
        drop(smtp);

        // 2. "Review certificate": each server's certificate is read, with
        //    no credentials.
        let (imap, smtp) = fakes();
        let (i, s) = (
            imap_server(imap.addr, Security::Tls),
            imap_server(smtp.addr, Security::StartTls),
        );
        let imap_der = probe_certificate(&i.host, i.port, i.security, Kind::Imap)
            .unwrap()
            .leaf;
        let smtp_der = probe_certificate(&s.host, s.port, s.security, Kind::Smtp)
            .unwrap()
            .leaf;
        assert_eq!(imap_der, der_of(CERT));
        assert_eq!(fingerprint_sha256(&smtp_der), fingerprint_sha256(&imap_der));
        for fake in [imap, smtp] {
            let seen = fake.finish();
            assert!(
                !seen
                    .iter()
                    .any(|l| l.contains("LOGIN") || l.to_ascii_uppercase().starts_with("AUTH")),
                "{seen:?}"
            );
        }

        // 3. Confirmed: the check passes with the pins, and both servers
        //    saw the sign-in.
        let (imap, smtp) = fakes();
        let (i, s) = (
            imap_server(imap.addr, Security::Tls),
            imap_server(smtp.addr, Security::StartTls),
        );
        let mut seen = Vec::new();
        verify_account(
            &i,
            &s,
            (Some(&imap_der), Some(&smtp_der)),
            ("alice", "alice"),
            "pw",
            &mut |c| seen.push(c),
        )
        .unwrap_or_else(|e| panic!("{e}: {:#}", e.source));
        assert_eq!(seen, Check::ALL);
        assert!(imap.finish().iter().any(|l| l.contains("LOGIN")));
        assert!(
            smtp.finish()
                .iter()
                .any(|l| l.to_ascii_uppercase().starts_with("AUTH"))
        );

        // 4. Saved with the account: the pins come back, and go with it.
        let db = Db::open(&temp).unwrap();
        let id = db
            .add_account(&NewAccount {
                email: "alice@example.org".into(),
                name: "Alice".into(),
                imap: i.clone(),
                smtp: s.clone(),
                save_sent: true,
                imap_user: String::new(),
                smtp_user: String::new(),
                smtp_no_auth: false,
            })
            .unwrap();
        let mut a = db.accounts().unwrap().remove(0);
        assert_eq!((a.imap_pin.as_deref(), a.smtp_pin.as_deref()), (None, None));
        a.imap_pin = Some(imap_der.clone());
        a.smtp_pin = Some(smtp_der.clone());
        assert!(db.update_account(&a).unwrap());
        let b = db.accounts().unwrap().remove(0);
        assert_eq!(b.id, id);
        assert_eq!(
            (
                usable(b.imap_pin.as_deref()).map(|p| p.der),
                usable(b.smtp_pin.as_deref()).map(|p| p.der),
            ),
            (Some(&imap_der[..]), Some(&smtp_der[..]))
        );
        // An edit of the name or login keeps both.
        let mut c = b.clone();
        c.name = "Alice B".into();
        c.imap_user = "alice".into();
        assert!(db.update_account(&c).unwrap());
        assert_eq!(
            db.accounts().unwrap()[0].imap_pin.as_deref(),
            Some(&imap_der[..])
        );
        db.remove_account(id).unwrap();
        assert!(db.accounts().unwrap().is_empty());
        drop(db);
        for suffix in ["", "-wal", "-shm"] {
            std::fs::remove_file(format!("{}{suffix}", temp.display())).ok();
        }
    }

    /// The whole self-hosted path on the TLS fakes, with nothing from the
    /// network: Find settings (injected DNS) fills the form, Test stops at the
    /// untrusted certificate (and nothing was sent), Review reads each
    /// certificate without credentials, confirming pins them, Test passes,
    /// Add saves the account with its pins, and the stored account signs in
    /// the way a worker does. A real server is not involved.
    #[test]
    fn find_test_review_confirm_test_add_and_sync_on_the_tls_fakes() {
        use crate::db::{Db, NewAccount};
        use crate::mail::discover::{self, Source, SrvRecord, discover};
        use crate::providers::Server;

        let fakes = || {
            (
                TlsFake::start(Proto::Imap, Mode::Implicit, CERT, KEY),
                TlsFake::start(Proto::Smtp, Mode::StartTls, CERT, KEY),
            )
        };
        let sent_credentials = |seen: &[String]| {
            seen.iter()
                .any(|l| l.contains("LOGIN") || l.to_ascii_uppercase().starts_with("AUTH"))
        };
        // 1. Find settings: the domain publishes SRV records (injected; the
        //    web lookup finds nothing), and the empty form is filled.
        let (imap, smtp) = fakes();
        let (imap_port, smtp_port) = (imap.addr.port(), smtp.addr.port());
        let srv = |name: &str| match name {
            "_imaps._tcp.example.org" => vec![SrvRecord {
                priority: 0,
                weight: 0,
                port: imap_port,
                target: "mail.example.org".into(),
            }],
            "_submission._tcp.example.org" => vec![SrvRecord {
                priority: 0,
                weight: 0,
                port: smtp_port,
                target: "mail.example.org".into(),
            }],
            _ => Vec::new(),
        };
        let fetch = |_: &str, _: &str| Err(crate::fetch::FetchError::Status(404));
        let out = discover(
            "alice@example.org",
            &discover::Lookups {
                srv: &srv,
                fetch: &fetch,
            },
        );
        assert_eq!(out.sources, [Source::Srv]);
        // The form takes what was found; the name would resolve to the
        // server, and here it is this machine.
        let (found_imap, found_smtp) = (
            out.found.imap.as_ref().unwrap().server.clone(),
            out.found.smtp.as_ref().unwrap().server.clone(),
        );
        assert_eq!(found_imap.host, "mail.example.org");
        assert_eq!(found_imap.security, Security::Tls);
        assert_eq!(found_smtp.security, Security::StartTls);
        let (i, s) = (
            Server {
                host: "localhost".into(),
                ..found_imap
            },
            Server {
                host: "localhost".into(),
                ..found_smtp
            },
        );

        // 2. Test connection: Windows does not trust the certificate. The
        //    check stops at the secure step with a certificate that can be
        //    reviewed, and no sign-in was sent.
        let mut steps = Vec::new();
        let e = crate::mail::verify_account(
            &i,
            &s,
            (None, None),
            ("alice@example.org", "alice@example.org"),
            "pw",
            &mut |c| steps.push(c),
        )
        .unwrap_err();
        assert_eq!(
            (e.step, e.class, e.special),
            (Check::Secure, Class::TlsCert, Special::None),
            "a certificate that can be reviewed"
        );
        assert_eq!(steps, [Check::Resolve, Check::Connect]);
        assert!(!sent_credentials(&imap.finish()));
        drop(smtp);

        // 3. Review certificate, for each server: the certificate is read
        //    and nothing else is sent.
        let (imap, smtp) = fakes();
        let (i, s) = (
            Server {
                port: imap.addr.port(),
                ..i
            },
            Server {
                port: smtp.addr.port(),
                ..s
            },
        );
        let imap_der = probe_certificate(&i.host, i.port, i.security, Kind::Imap)
            .unwrap()
            .leaf;
        let smtp_der = probe_certificate(&s.host, s.port, s.security, Kind::Smtp)
            .unwrap()
            .leaf;
        assert_eq!(imap_der, der_of(CERT));
        let shown = fingerprint_sha256(&imap_der);
        assert_eq!(shown.len(), 32 * 3 - 1, "the whole fingerprint is shown");
        for fake in [imap, smtp] {
            assert!(!sent_credentials(&fake.finish()));
        }

        // 4. Confirmed: Test connection passes with both pins, every step.
        let (imap, smtp) = fakes();
        let (i, s) = (
            Server {
                port: imap.addr.port(),
                ..i
            },
            Server {
                port: smtp.addr.port(),
                ..s
            },
        );
        let mut steps = Vec::new();
        crate::mail::verify_account(
            &i,
            &s,
            (Some(&imap_der), Some(&smtp_der)),
            ("alice@example.org", "alice@example.org"),
            "pw",
            &mut |c| steps.push(c),
        )
        .unwrap_or_else(|e| panic!("{e}: {:#}", e.source));
        assert_eq!(steps, Check::ALL);
        assert!(sent_credentials(&imap.finish()));
        assert!(sent_credentials(&smtp.finish()));

        // 5. Add: saved with the pins and the effective sign-in (the email).
        let temp =
            std::env::temp_dir().join(format!("komorebi-find-e2e-{}.sqlite3", std::process::id()));
        let clean = || {
            for suffix in ["", "-wal", "-shm"] {
                std::fs::remove_file(format!("{}{suffix}", temp.display())).ok();
            }
        };
        clean();
        let db = Db::open(&temp).unwrap();
        db.add_account(&NewAccount {
            email: "alice@example.org".into(),
            name: "Alice".into(),
            imap: i.clone(),
            smtp: s.clone(),
            save_sent: true,
            imap_user: String::new(),
            smtp_user: String::new(),
            smtp_no_auth: false,
        })
        .unwrap();
        let mut account = db.accounts().unwrap().remove(0);
        account.imap_pin = Some(imap_der.clone());
        account.smtp_pin = Some(smtp_der.clone());
        assert!(db.update_account(&account).unwrap());
        let account = db.accounts().unwrap().remove(0);
        assert_eq!(account.imap_login(), "alice@example.org");
        assert_eq!(
            usable(account.imap_pin.as_deref()).map(|p| p.der),
            Some(&imap_der[..])
        );

        // 6. Sync starts: the worker's connection, from the stored account,
        //    signs in to the pinned server.
        let imap = TlsFake::start(Proto::Imap, Mode::Implicit, CERT, KEY);
        let server = Server {
            port: imap.addr.port(),
            ..account.imap.clone()
        };
        Imap::connect(
            &server,
            account.imap_pin.as_deref(),
            account.imap_login(),
            "pw",
        )
        .unwrap_or_else(|e| panic!("{e:#}"));
        let seen = imap.finish();
        assert!(
            seen.iter()
                .any(|l| l.contains("LOGIN") && l.contains("alice@example.org")),
            "{seen:?}"
        );
        drop(db);
        clean();
    }
}
