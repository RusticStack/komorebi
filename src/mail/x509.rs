//! A small, panic-free reader of X.509 certificates (DER): just enough to
//! find a certificate's issuer, subject and public key (SubjectPublicKeyInfo)
//! for certificate pinning (`mail::pin`) and the review prompt.
//!
//! It never decides trust by itself: the server key pin compares the SHA-256
//! of the key a TLS handshake proved, and the CA pin hands the CA certificate
//! to Windows, which checks the signatures. Input is capped
//! ([`MAX_DER`]), lengths are checked before every slice, the reader walks
//! one level at a time (no recursion) and a list stops at [`MAX_ITEMS`].

/// The longest certificate read (DER bytes).
pub const MAX_DER: usize = 16 * 1024;
/// The most values read from one constructed value.
const MAX_ITEMS: usize = 256;

/// A DER value: its tag and contents, and the whole encoding.
pub(crate) struct Der<'a> {
    pub tag: u8,
    pub body: &'a [u8],
    pub whole: &'a [u8],
}

/// The first DER value of `data` and what follows it. Definite lengths of
/// at most three bytes only; indefinite (`0x80`) and long tag numbers are
/// refused.
pub(crate) fn take(data: &[u8]) -> Option<(Der<'_>, &[u8])> {
    let tag = *data.first()?;
    if tag & 0x1f == 0x1f {
        return None; // long tag numbers do not occur here
    }
    let first = *data.get(1)?;
    let (len, header) = if first < 0x80 {
        (first as usize, 2)
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 3 {
            return None;
        }
        let bytes = data.get(2..2 + n)?;
        (
            bytes.iter().fold(0usize, |l, &b| (l << 8) | b as usize),
            2 + n,
        )
    };
    let end = header.checked_add(len)?;
    let whole = data.get(..end)?;
    Some((
        Der {
            tag,
            body: whole.get(header..)?,
            whole,
        },
        data.get(end..)?,
    ))
}

/// Every value of `data`, which must hold whole values only.
pub(crate) fn items(mut data: &[u8]) -> Option<Vec<Der<'_>>> {
    let mut out = Vec::new();
    while !data.is_empty() {
        let (item, rest) = take(data)?;
        out.push(item);
        data = rest;
        if out.len() > MAX_ITEMS {
            return None;
        }
    }
    Some(out)
}

/// The fields of a certificate's TBSCertificate that pinning uses.
pub(crate) struct Fields<'a> {
    pub issuer: Der<'a>,
    pub subject: Der<'a>,
    /// The whole SubjectPublicKeyInfo (algorithm and key), as encoded.
    pub spki: &'a [u8],
}

/// Read the fields of the certificate `der`, if it is one.
pub(crate) fn fields(der: &[u8]) -> Option<Fields<'_>> {
    if der.is_empty() || der.len() > MAX_DER {
        return None;
    }
    let (cert, trailing) = take(der)?;
    if cert.tag != 0x30 || !trailing.is_empty() {
        return None;
    }
    let (tbs, _) = take(cert.body)?;
    if tbs.tag != 0x30 {
        return None;
    }
    let mut list = items(tbs.body)?.into_iter();
    let mut next = list.next()?;
    if next.tag == 0xa0 {
        next = list.next()?; // the version; `next` was the serial
    }
    if next.tag != 0x02 {
        return None;
    }
    let _algorithm = list.next().filter(|d| d.tag == 0x30)?;
    let issuer = list.next().filter(|d| d.tag == 0x30)?;
    let _validity = list.next().filter(|d| d.tag == 0x30)?;
    let subject = list.next().filter(|d| d.tag == 0x30)?;
    let key = list.next().filter(|d| d.tag == 0x30)?;
    // SubjectPublicKeyInfo: an algorithm and a BIT STRING.
    let parts = items(key.body)?;
    if parts.len() != 2 || parts[0].tag != 0x30 || parts[1].tag != 0x03 {
        return None;
    }
    Some(Fields {
        issuer,
        subject,
        spki: key.whole,
    })
}

/// The encoded SubjectPublicKeyInfo of `der`.
pub fn spki(der: &[u8]) -> Option<&[u8]> {
    fields(der).map(|f| f.spki)
}

/// The SHA-256 of the SubjectPublicKeyInfo of `der` (the usual "SPKI pin").
pub fn spki_sha256(der: &[u8]) -> Option<[u8; 32]> {
    spki(der).map(super::pin::sha256)
}

/// Whether `issuer` (a certificate) names `leaf`'s issuer as its subject,
/// and is not `leaf` itself. Names only: the signature is Windows' to check.
pub fn names_issuer_of(issuer: &[u8], leaf: &[u8]) -> bool {
    match (fields(issuer), fields(leaf)) {
        (Some(ca), Some(l)) => {
            issuer != leaf
                && ca.subject.whole == l.issuer.whole
                && l.issuer.whole != l.subject.whole
        }
        _ => false,
    }
}

/// Whether `der` names itself as its issuer.
pub fn self_issued(der: &[u8]) -> bool {
    fields(der).is_some_and(|f| f.issuer.whole == f.subject.whole)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::fake_tls::der_of;

    const TEST: &[u8] = include_bytes!("../../tests/fixtures/tls/test-cert.pem");
    const RENEWED: &[u8] = include_bytes!("../../tests/fixtures/tls/renewed-cert.pem");
    const OTHER: &[u8] = include_bytes!("../../tests/fixtures/tls/other-cert.pem");
    const CA: &[u8] = include_bytes!("../../tests/fixtures/tls/ca-cert.pem");
    const CA_LEAF: &[u8] = include_bytes!("../../tests/fixtures/tls/ca-leaf-cert.pem");

    #[test]
    fn the_key_survives_a_renewal_and_differs_from_another_key() {
        let (test, renewed, other) = (der_of(TEST), der_of(RENEWED), der_of(OTHER));
        assert_ne!(test, renewed);
        assert_eq!(spki_sha256(&test), spki_sha256(&renewed));
        assert_ne!(spki_sha256(&test), spki_sha256(&other));
        assert!(spki(&test).is_some_and(|k| k.len() > 200 && k[0] == 0x30));
    }

    #[test]
    fn the_issuer_is_found_by_name() {
        let (ca, leaf, test) = (der_of(CA), der_of(CA_LEAF), der_of(TEST));
        assert!(names_issuer_of(&ca, &leaf));
        assert!(!names_issuer_of(&leaf, &ca));
        assert!(!names_issuer_of(&test, &test));
        assert!(!names_issuer_of(&ca, &test));
        assert!(self_issued(&test) && self_issued(&ca) && !self_issued(&leaf));
    }

    #[test]
    fn hostile_der_is_refused_without_panicking() {
        let good = der_of(TEST);
        // Every truncation, and every single byte changed, reads or refuses.
        for n in 0..good.len() {
            assert!(fields(&good[..n]).is_none(), "truncated at {n}");
        }
        for i in 0..good.len().min(600) {
            let mut bad = good.clone();
            bad[i] ^= 0xff;
            let _ = fields(&bad);
        }
        let cases: &[&[u8]] = &[
            &[],
            &[0x30],
            &[0x30, 0x80, 0, 0],                   // indefinite length
            &[0x30, 0x84, 0xff, 0xff, 0xff, 0xff], // four length bytes
            &[0x30, 0x83, 0xff, 0xff, 0xff],       // longer than the data
            &[0x1f, 0x01, 0x00],                   // long tag number
            &[0x30, 0x02, 0x30, 0x00],             // empty TBS
            &[0x30, 0x03, 0x30, 0x01, 0x02],
        ];
        for case in cases {
            assert!(fields(case).is_none(), "{case:?}");
            assert!(spki_sha256(case).is_none());
        }
        // Deep nesting is walked one level at a time.
        let mut deep = vec![0x30u8; 4000];
        deep.extend([0u8; 4000]);
        assert!(fields(&deep).is_none());
        // Too many items in one value.
        let mut many = vec![0x30, 0x82, 0x02, 0x02];
        for _ in 0..257 {
            many.extend([0x05, 0x00]);
        }
        assert!(items(&many[4..]).is_none());
        // Trailing bytes after the certificate.
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(fields(&trailing).is_none());
        // Over the cap.
        assert!(fields(&vec![0x30; MAX_DER + 1]).is_none());
    }
}
