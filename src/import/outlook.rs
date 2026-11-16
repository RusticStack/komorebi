//! Import from Outlook: the accounts and signatures classic Outlook keeps on
//! this PC, read so the setup flow can offer them. Mail is not imported; the
//! account downloads it again from its server.
//!
//! - **Accounts** come from the MAPI profiles in the registry,
//!   `HKCU\Software\Microsoft\Office\<16.0|15.0>\Outlook\Profiles\<profile>\9375CFF0413111d3B88A00104B2A6676\<n>`
//!   (Outlook 2013 and later) and
//!   `HKCU\Software\Microsoft\Windows NT\CurrentVersion\Windows Messaging Subsystem\Profiles\<profile>\9375CFF0413111d3B88A00104B2A6676\<n>`
//!   (Outlook 2010 and earlier). Each account key holds values named
//!   `Display Name`, `Email`, `Account Name`, `IMAP Server` / `POP3 Server`,
//!   `<proto> Port`, `<proto> Use SSL`, `<proto> Secure Connection`,
//!   `<proto> User`, `SMTP Server`, `SMTP Port`, `SMTP Use SSL`,
//!   `SMTP Secure Connection`, `SMTP Use Auth`, `SMTP User`,
//!   `New Signature` and `Reply-Forward Signature`. Strings are REG_SZ or
//!   REG_BINARY UTF-16LE with a terminating NUL, numbers REG_DWORD. Value
//!   names holding "Password" are never read: the Windows layer lists value
//!   names first and skips them before any data is queried.
//! - An account with neither an IMAP nor a POP3 server but an address is an
//!   Exchange, Microsoft 365 or Outlook.com account ([`Kind::Exchange`]):
//!   it has no IMAP settings to take, so the app maps it to a provider.
//! - **Signatures** are the `.htm` files in `%APPDATA%\Microsoft\Signatures`
//!   (the folder's name is localised, as in "Assinaturas"; Office keeps it in
//!   `HKCU\Software\Microsoft\Office\<ver>\Common\General`, value
//!   `Signatures`), each with its pictures in a `<name>_files` folder (also
//!   localised: `<name>_ficheiros`, `<name>-Dateien` and so on). The HTML is
//!   decoded from its declared charset and returned as it is: converting it
//!   is the app's work.
//! - **The new Outlook** (olk.exe) keeps accounts and signatures in the
//!   cloud: when it is the only Outlook here, [`Found::new_outlook_only`]
//!   says so and there is nothing to import.
//!
//! The parsing functions take plain data (a map of registry values, a
//! directory listing), so tests run without Outlook; [`discover`] is the thin
//! Windows layer that reads the registry and the files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::mail::rich;
use crate::providers::{Security, Server};

/// The registry key that holds a profile's mail accounts.
pub const ACCOUNTS_KEY: &str = "9375CFF0413111d3B88A00104B2A6676";

/// The Office versions whose profiles are read, newest first (2016 and
/// later share 16.0; 15.0 is 2013).
pub const VERSIONS: [&str; 2] = ["16.0", "15.0"];

/// The largest signature file read (a signature's HTML is a few kilobytes;
/// anything larger is not one).
pub const MAX_SIGNATURE_BYTES: u64 = 1024 * 1024;

/// Picture folder suffixes Office writes next to a saved signature, by
/// display language ("<name>_files" in English).
pub const IMAGE_FOLDER_SUFFIXES: &[&str] = &[
    "_files",
    "_ficheiros",
    "_arquivos",
    "-Dateien",
    "_fichiers",
    "_archivos",
    "_bestanden",
    "_file",
    "_pliki",
    "_soubory",
    "-filer",
    "_filer",
    "_tiedostot",
    "-tiedostot",
    "_fájlok",
    "_dosyalar",
];

/// A registry value as read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegValue {
    /// REG_SZ or REG_EXPAND_SZ.
    Sz(String),
    /// REG_BINARY.
    Binary(Vec<u8>),
    /// REG_DWORD.
    Dword(u32),
    /// Any other type.
    Other,
}

/// An account key's values by name.
pub type Values = BTreeMap<String, RegValue>;

/// Whether a value name holds a secret, never read or kept.
pub fn is_secret(name: &str) -> bool {
    name.to_ascii_lowercase().contains("password")
}

/// What kind of account Outlook had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Imap,
    /// POP3: Komorebi is IMAP only, so the app offers the provider's IMAP
    /// settings to confirm.
    Pop,
    /// Exchange, Microsoft 365 or Outlook.com (MAPI, Exchange ActiveSync or
    /// the old HTTP access): no IMAP settings.
    Exchange,
}

/// An account found in an Outlook profile. No password: Outlook's are never
/// read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundAccount {
    pub kind: Kind,
    /// The Outlook profile it is in.
    pub profile: String,
    /// The account's name in Outlook (often the address).
    pub account_name: String,
    /// The sender name.
    pub display_name: String,
    pub email: String,
    /// The incoming server (IMAP or POP3).
    pub incoming: Option<Server>,
    /// The incoming sign-in name; empty when Outlook had none.
    pub incoming_user: String,
    pub smtp: Option<Server>,
    /// The sending server's own sign-in name; empty when it signs in as the
    /// incoming server does.
    pub smtp_user: String,
    /// The signature (its file name without `.htm`) Outlook adds to new mail.
    pub new_signature: Option<String>,
    /// The one it adds to replies and forwards.
    pub reply_signature: Option<String>,
}

/// A signature found in Outlook's signatures folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundSignature {
    /// Its name in Outlook: the file name without `.htm`.
    pub name: String,
    /// The HTML, decoded from its charset, not converted.
    pub html: String,
    /// The charset it was decoded from.
    pub charset: String,
    /// Its pictures on this PC, as the HTML refers to them.
    pub images: Vec<PathBuf>,
}

/// Everything found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub accounts: Vec<FoundAccount>,
    pub signatures: Vec<FoundSignature>,
    /// Only the new Outlook is here: its accounts and signatures are in the
    /// cloud, so nothing local can be imported.
    pub new_outlook_only: bool,
}

impl Found {
    /// Whether there is anything to offer.
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty() && self.signatures.is_empty()
    }
}

// ---- registry values ------------------------------------------------------

fn get<'a>(values: &'a Values, name: &str) -> Option<&'a RegValue> {
    if is_secret(name) {
        return None;
    }
    values.get(name).or_else(|| {
        values
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
    })
}

/// REG_BINARY text: UTF-16LE (what Outlook writes) up to its NUL, or, when
/// the bytes are not UTF-16 (an odd length, Outlook 2007's ANSI values),
/// single-byte text up to its NUL.
pub fn binary_text(bytes: &[u8]) -> String {
    // UTF-16 text has a NUL in its first unit's high byte (Latin text) or
    // ends in a two-byte NUL at an even offset; ANSI text ends in one NUL.
    let first_nul = bytes.iter().position(|&b| b == 0);
    let looks_wide = bytes.len().is_multiple_of(2)
        && (first_nul == Some(1)
            || (bytes.ends_with(&[0, 0]) && first_nul.is_some_and(|p| p.is_multiple_of(2))));
    if looks_wide {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        if let Ok(s) = String::from_utf16(&units) {
            return s;
        }
    }
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let narrow = &bytes[..end];
    match std::str::from_utf8(narrow) {
        Ok(s) => s.to_string(),
        Err(_) => narrow.iter().map(|&b| char::from(b)).collect(),
    }
}

/// A string value, trimmed; `None` when missing or blank.
pub fn text(values: &Values, name: &str) -> Option<String> {
    let s = match get(values, name)? {
        RegValue::Sz(s) => s.trim_end_matches('\0').to_string(),
        RegValue::Binary(b) => binary_text(b),
        RegValue::Dword(_) | RegValue::Other => return None,
    };
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// A number value: REG_DWORD, four bytes of REG_BINARY, or digits.
pub fn dword(values: &Values, name: &str) -> Option<u32> {
    match get(values, name)? {
        RegValue::Dword(d) => Some(*d),
        RegValue::Binary(b) if b.len() == 4 => Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        RegValue::Sz(s) => s.trim().trim_end_matches('\0').parse().ok(),
        _ => None,
    }
}

fn looks_like_address(s: &str) -> bool {
    s.rsplit_once('@')
        .is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.contains(' '))
}

/// A server from `<prefix> Server`, `Port`, `Secure Connection` (0 none,
/// 1 SSL/TLS, 2 STARTTLS, 3 automatic) and `Use SSL`. Komorebi connects
/// with TLS or STARTTLS only: an explicit choice wins, then the port's usual
/// meaning, then `Use SSL`.
fn server(values: &Values, prefix: &str, plain: u16, tls: u16) -> Option<Server> {
    let host = text(values, &format!("{prefix} Server"))?;
    let port = dword(values, &format!("{prefix} Port"))
        .and_then(|p| u16::try_from(p).ok())
        .filter(|&p| p != 0);
    let secure = dword(values, &format!("{prefix} Secure Connection"));
    let ssl = dword(values, &format!("{prefix} Use SSL")).is_some_and(|v| v != 0);
    let security = match (secure, port) {
        (Some(1), _) => Security::Tls,
        (Some(2), _) => Security::StartTls,
        (_, Some(993 | 995 | 465)) => Security::Tls,
        (_, Some(143 | 110 | 587 | 25)) => Security::StartTls,
        _ if ssl => Security::Tls,
        _ => Security::StartTls,
    };
    let port = port.unwrap_or(match security {
        Security::Tls => tls,
        Security::StartTls => plain,
    });
    Some(Server {
        host,
        port,
        security,
    })
}

/// One account key's values as an account; `None` for a key that is not a
/// mail account (an address book, a key with no address).
pub fn parse_account(profile: &str, values: &Values) -> Option<FoundAccount> {
    if text(values, "LDAP Server").is_some() {
        return None;
    }
    let account_name = text(values, "Account Name").unwrap_or_default();
    let email = text(values, "Email")
        .filter(|e| looks_like_address(e))
        .or_else(|| text(values, "SMTP Email Address").filter(|e| looks_like_address(e)))
        .or_else(|| Some(account_name.clone()).filter(|e| looks_like_address(e)))?;
    let (kind, incoming, user) = if let Some(s) = server(values, "IMAP", 143, 993) {
        (Kind::Imap, Some(s), text(values, "IMAP User"))
    } else if let Some(s) = server(values, "POP3", 110, 995) {
        (Kind::Pop, Some(s), text(values, "POP3 User"))
    } else {
        (Kind::Exchange, None, None)
    };
    let smtp = match kind {
        Kind::Exchange => None,
        _ => server(values, "SMTP", 25, 465),
    };
    let smtp_user = if dword(values, "SMTP Use Auth").unwrap_or(1) != 0 {
        text(values, "SMTP User").unwrap_or_default()
    } else {
        String::new()
    };
    Some(FoundAccount {
        kind,
        profile: profile.to_string(),
        display_name: text(values, "Display Name").unwrap_or_default(),
        account_name,
        email,
        incoming,
        incoming_user: user.unwrap_or_default(),
        smtp,
        smtp_user,
        new_signature: text(values, "New Signature"),
        reply_signature: text(values, "Reply-Forward Signature"),
    })
}

/// The accounts of several profiles' keys (`(profile, values)`, newest
/// Office first), each address once: the first seen wins. A global default
/// signature (`Common\MailSettings`, `NewSignature` / `ReplySignature`)
/// fills in for an account that names none.
pub fn collect_accounts(
    keys: &[(String, Values)],
    global_new: Option<&str>,
    global_reply: Option<&str>,
) -> Vec<FoundAccount> {
    let mut out: Vec<FoundAccount> = Vec::new();
    for (profile, values) in keys {
        let Some(mut a) = parse_account(profile, values) else {
            continue;
        };
        if out.iter().any(|b| b.email.eq_ignore_ascii_case(&a.email)) {
            continue;
        }
        if a.new_signature.is_none() {
            a.new_signature = global_new.map(str::to_string).filter(|s| !s.is_empty());
        }
        if a.reply_signature.is_none() {
            a.reply_signature = global_reply.map(str::to_string).filter(|s| !s.is_empty());
        }
        out.push(a);
    }
    out
}

/// Whether `email` is already an account here (case aside).
pub fn already_added<'a>(email: &str, existing: impl IntoIterator<Item = &'a str>) -> bool {
    existing
        .into_iter()
        .any(|e| e.trim().eq_ignore_ascii_case(email.trim()))
}

// ---- signatures -----------------------------------------------------------

/// The charset a signature's HTML declares (`<meta charset=…>` or
/// `content="text/html; charset=…"`), lower case.
pub fn declared_charset(head: &[u8]) -> Option<String> {
    let head = &head[..head.len().min(4096)];
    let lower: Vec<u8> = head.iter().map(u8::to_ascii_lowercase).collect();
    let at = lower.windows(8).position(|w| w == b"charset=")? + 8;
    let rest = &lower[at..];
    let rest = rest
        .strip_prefix(b"\"")
        .or_else(|| rest.strip_prefix(b"'"))
        .unwrap_or(rest);
    let name: Vec<u8> = rest
        .iter()
        .copied()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b':' | b'.'))
        .collect();
    (!name.is_empty()).then(|| String::from_utf8_lossy(&name).into_owned())
}

fn utf16(bytes: &[u8], le: bool) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| {
            if le {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

/// A signature file's text and the charset it was read as: a byte order
/// mark first, then the declared charset, then UTF-8 if it is valid, else
/// Windows-1252 (what Outlook saves in western languages).
pub fn decode_html(bytes: &[u8]) -> (String, String) {
    if let Some(rest) = bytes.strip_prefix(b"\xef\xbb\xbf") {
        return (String::from_utf8_lossy(rest).into_owned(), "utf-8".into());
    }
    if let Some(rest) = bytes.strip_prefix(b"\xff\xfe") {
        return (utf16(rest, true), "utf-16le".into());
    }
    if let Some(rest) = bytes.strip_prefix(b"\xfe\xff") {
        return (utf16(rest, false), "utf-16be".into());
    }
    let declared = declared_charset(bytes);
    if let Some(cs) = &declared {
        if matches!(cs.as_str(), "utf-8" | "utf8") {
            return (String::from_utf8_lossy(bytes).into_owned(), "utf-8".into());
        }
        if let Some(decode) = mail_parser::decoders::charsets::map::charset_decoder(cs.as_bytes()) {
            return (decode(bytes), cs.clone());
        }
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return (s.to_string(), "utf-8".into());
    }
    let cp1252 = mail_parser::decoders::charsets::map::charset_decoder(b"windows-1252");
    match cp1252 {
        Some(decode) => (decode(bytes), "windows-1252".into()),
        None => (
            bytes.iter().map(|&b| char::from(b)).collect(),
            "iso-8859-1".into(),
        ),
    }
}

/// The relative paths a signature's HTML refers to by `src=` (and VML's
/// `o:href=`), in order, `%20` and friends decoded, `\` as `/`. Absolute
/// URLs and `cid:` are left out.
pub fn referenced_paths(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut out: Vec<String> = Vec::new();
    for needle in ["src=", "o:href="] {
        let mut from = 0;
        while let Some(i) = lower[from..].find(needle) {
            let start = from + i + needle.len();
            from = start;
            let rest = &html[start..];
            let (quote, body) = match rest.chars().next() {
                Some(q @ ('"' | '\'')) => (Some(q), &rest[1..]),
                _ => (None, rest),
            };
            let end = body
                .find(|c: char| match quote {
                    Some(q) => c == q,
                    None => c.is_whitespace() || c == '>',
                })
                .unwrap_or(body.len());
            let raw = &body[..end];
            if raw.contains(':') || raw.starts_with('/') || raw.starts_with('\\') || raw.is_empty()
            {
                continue;
            }
            let path = percent_decode(raw).replace('\\', "/");
            if path.split('/').any(|p| p == "..") {
                continue;
            }
            if !out.contains(&path) {
                out.push(path);
            }
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

fn is_image(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp"]
        .iter()
        .any(|e| lower.ends_with(e))
}

/// The signatures in a signatures folder listing. `listing` holds the
/// folder's files as paths relative to it with `/` (the top level, and one
/// level inside its sub-folders); `read` gives a file's bytes. A signature
/// is a top-level `.htm` file; its pictures are the image files its HTML
/// refers to, or, when it refers to none that exist, the images in its
/// picture folder (`<name>_files` or a localised name).
pub fn signatures_in(
    dir: &Path,
    listing: &[String],
    read: impl Fn(&str) -> Option<Vec<u8>>,
) -> Vec<FoundSignature> {
    let mut found: Vec<FoundSignature> = Vec::new();
    let mut files: Vec<&String> = listing
        .iter()
        .filter(|p| !p.contains('/'))
        .filter(|p| {
            let l = p.to_ascii_lowercase();
            l.ends_with(".htm") || l.ends_with(".html")
        })
        .collect();
    files.sort_by_key(|p| p.to_lowercase());
    for file in files {
        let name = match file.rsplit_once('.') {
            Some((stem, _)) if !stem.trim().is_empty() => stem.to_string(),
            _ => continue,
        };
        let Some(bytes) = read(file) else {
            continue;
        };
        let (html, charset) = decode_html(&bytes);
        let exists = |rel: &str| listing.iter().any(|p| p.eq_ignore_ascii_case(rel));
        let mut images: Vec<PathBuf> = referenced_paths(&html)
            .into_iter()
            .filter(|p| is_image(p) && exists(p))
            .map(|p| dir.join(p.replace('/', std::path::MAIN_SEPARATOR_STR)))
            .collect();
        if images.is_empty() {
            let folders: Vec<String> = IMAGE_FOLDER_SUFFIXES
                .iter()
                .map(|s| format!("{name}{s}/").to_lowercase())
                .collect();
            images = listing
                .iter()
                .filter(|p| {
                    let l = p.to_lowercase();
                    folders.iter().any(|f| l.starts_with(f.as_str())) && is_image(p)
                })
                .map(|p| dir.join(p.replace('/', std::path::MAIN_SEPARATOR_STR)))
                .collect();
            images.sort();
        }
        found.push(FoundSignature {
            name,
            html,
            charset,
            images,
        });
    }
    found
}

/// A found signature as Komorebi keeps it: its HTML read with its layout
/// (tables, merged cells, colours) and the pictures on this PC it refers to
/// (the files in [`FoundSignature::images`], read by `read`), carried in the
/// signature itself. A picture from the web is left out, never fetched; one
/// in the HTML itself (`data:`) is kept. `None` when nothing readable is
/// left. Reads files: call it off the UI thread.
pub fn to_signature(
    sig: &FoundSignature,
    read: &dyn Fn(&Path) -> Option<Vec<u8>>,
) -> Option<(rich::Doc, rich::Images)> {
    let known = rich::Images::default();
    // Only the signature's own picture files are read, never another path
    // the HTML names.
    let nothing = |_: &Path| None;
    let mut r = rich::images::Resolver::new(&known, &nothing);
    let doc = rich::import::lossy_with_images(&sig.html, &mut |f| {
        if f.src.trim_start().starts_with("data:") {
            return r.resolve(f);
        }
        let bytes = read(local_picture(sig, &f.src)?)?;
        r.resolve_bytes(bytes, f)
    })?;
    Some((doc, r.found))
}

/// The file of `sig.images` a relative `src` (such as
/// `Work_files/image001.png`) names, matched on its last parts.
fn local_picture<'a>(sig: &'a FoundSignature, src: &str) -> Option<&'a Path> {
    let raw = src.trim();
    if raw.is_empty() || raw.contains(':') || raw.starts_with('/') || raw.starts_with('\\') {
        return None;
    }
    let rel = percent_decode(raw).replace('\\', "/");
    let want: Vec<String> = rel
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .map(str::to_lowercase)
        .collect();
    if want.is_empty() || want.iter().any(|p| p == "..") {
        return None;
    }
    sig.images.iter().map(PathBuf::as_path).find(|p| {
        let tail: Vec<String> = p
            .components()
            .rev()
            .take(want.len())
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .collect();
        tail.len() == want.len() && tail.iter().rev().eq(want.iter())
    })
}

/// How many pictures a signature's HTML takes from the web (an `<img>`
/// with an `http` or `https` address): they are never fetched, so the
/// import list says so.
pub fn web_pictures(html: &str) -> usize {
    let lower = html.to_ascii_lowercase();
    let mut n = 0;
    let mut from = 0;
    while let Some(i) = lower.get(from..).and_then(|s| s.find("<img")) {
        let start = from + i;
        let end = lower
            .get(start..)
            .and_then(|s| s.find('>'))
            .map_or(lower.len(), |e| start + e);
        let tag: String = lower
            .get(start..end)
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
            .collect();
        if tag.contains("src=http:") || tag.contains("src=https:") {
            n += 1;
        }
        from = end.max(start + 4);
    }
    n
}

// ---- the Windows layer ----------------------------------------------------

/// Read what classic Outlook keeps on this PC: its accounts (no passwords)
/// and its signatures. Read-only; on another system, nothing.
pub fn discover() -> Found {
    #[cfg(windows)]
    {
        win::discover()
    }
    #[cfg(not(windows))]
    {
        Found::default()
    }
}

/// [`discover`]'s signatures alone.
pub fn signatures() -> Vec<FoundSignature> {
    discover().signatures
}

#[cfg(windows)]
mod win {
    use std::path::{Path, PathBuf};
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, REG_BINARY, REG_DWORD, REG_EXPAND_SZ, REG_SZ,
        RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, RegQueryValueExW,
    };

    use super::*;

    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the handle came from RegOpenKeyExW and is closed once.
            unsafe { RegCloseKey(self.0) };
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn open(parent: HKEY, path: &str) -> Option<Key> {
        let sub = wide(path);
        let mut key: HKEY = null_mut();
        // SAFETY: sub is NUL-terminated; key receives the handle. Read
        // access only.
        let rc = unsafe { RegOpenKeyExW(parent, sub.as_ptr(), 0, KEY_READ, &mut key) };
        (rc == ERROR_SUCCESS).then_some(Key(key))
    }

    fn subkeys(key: &Key) -> Vec<String> {
        let mut out = Vec::new();
        for index in 0..4096u32 {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            // SAFETY: name holds len UTF-16 units; the other outputs are
            // not asked for.
            let rc = unsafe {
                RegEnumKeyExW(
                    key.0,
                    index,
                    name.as_mut_ptr(),
                    &mut len,
                    null(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                )
            };
            if rc != ERROR_SUCCESS {
                break;
            }
            out.push(String::from_utf16_lossy(&name[..len as usize]));
        }
        out
    }

    /// The key's value names only: no data is read here.
    fn value_names(key: &Key) -> Vec<String> {
        let mut out = Vec::new();
        for index in 0..4096u32 {
            let mut name = [0u16; 16384];
            let mut len = name.len() as u32;
            // SAFETY: name holds len UTF-16 units; lpdata and lpcbdata are
            // null, so no value data is read.
            let rc = unsafe {
                RegEnumValueW(
                    key.0,
                    index,
                    name.as_mut_ptr(),
                    &mut len,
                    null(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                )
            };
            if rc != ERROR_SUCCESS {
                break;
            }
            out.push(String::from_utf16_lossy(&name[..len as usize]));
        }
        out
    }

    /// One value's data; never called for a secret's name.
    fn value(key: &Key, name: &str) -> Option<RegValue> {
        if is_secret(name) {
            return None;
        }
        let wname = wide(name);
        let mut kind = 0u32;
        let mut size = 0u32;
        // SAFETY: a size-only query.
        let rc = unsafe {
            RegQueryValueExW(
                key.0,
                wname.as_ptr(),
                null(),
                &mut kind,
                null_mut(),
                &mut size,
            )
        };
        if rc != ERROR_SUCCESS || size as usize > 64 * 1024 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        // SAFETY: data holds size bytes.
        let rc = unsafe {
            RegQueryValueExW(
                key.0,
                wname.as_ptr(),
                null(),
                &mut kind,
                data.as_mut_ptr(),
                &mut size,
            )
        };
        if rc != ERROR_SUCCESS {
            return None;
        }
        data.truncate(size as usize);
        Some(match kind {
            REG_SZ | REG_EXPAND_SZ => RegValue::Sz(binary_text(&data)),
            REG_BINARY => RegValue::Binary(data),
            REG_DWORD if data.len() >= 4 => {
                RegValue::Dword(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
            }
            _ => RegValue::Other,
        })
    }

    fn values(key: &Key) -> Values {
        value_names(key)
            .into_iter()
            .filter(|n| !is_secret(n))
            .filter_map(|n| value(key, &n).map(|v| (n, v)))
            .collect()
    }

    fn string(path: &str, name: &str) -> Option<String> {
        let key = open(HKEY_CURRENT_USER, path)?;
        match value(&key, name)? {
            RegValue::Sz(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
            RegValue::Binary(b) => Some(binary_text(&b)).filter(|s| !s.is_empty()),
            _ => None,
        }
    }

    /// The account keys of every profile under `profiles`.
    fn profile_keys(profiles: &str, out: &mut Vec<(String, Values)>) {
        let Some(root) = open(HKEY_CURRENT_USER, profiles) else {
            return;
        };
        for profile in subkeys(&root) {
            let Some(accounts) = open(root.0, &format!("{profile}\\{ACCOUNTS_KEY}")) else {
                continue;
            };
            let mut ids = subkeys(&accounts);
            ids.sort();
            for id in ids {
                if let Some(key) = open(accounts.0, &id) {
                    out.push((profile.clone(), values(&key)));
                }
            }
        }
    }

    fn classic_installed() -> bool {
        use windows_sys::Win32::System::Registry::HKEY_LOCAL_MACHINE;
        let app = "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\OUTLOOK.EXE";
        open(HKEY_LOCAL_MACHINE, app).is_some()
            || open(
                HKEY_LOCAL_MACHINE,
                "Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\App Paths\\OUTLOOK.EXE",
            )
            .is_some()
    }

    /// The new Outlook: its package's data folder, or the olk.exe alias.
    fn new_outlook_installed() -> bool {
        dirs::data_local_dir().is_some_and(|local| {
            local
                .join("Packages")
                .join("Microsoft.OutlookForWindows_8wekyb3d8bbwe")
                .is_dir()
                || local
                    .join("Microsoft")
                    .join("WindowsApps")
                    .join("olk.exe")
                    .exists()
        })
    }

    /// The folder's files relative to it, with `/`: the top level and one
    /// level of sub-folders.
    fn listing(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let Ok(top) = std::fs::read_dir(dir) else {
            return out;
        };
        for entry in top.flatten().take(2048) {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if let Ok(inner) = std::fs::read_dir(entry.path()) {
                    for f in inner.flatten().take(512) {
                        if f.file_type().is_ok_and(|k| k.is_file()) {
                            out.push(format!("{name}/{}", f.file_name().to_string_lossy()));
                        }
                    }
                }
            } else if kind.is_file() {
                out.push(name);
            }
        }
        out
    }

    fn signature_dirs() -> Vec<PathBuf> {
        let Some(roaming) = dirs::config_dir() else {
            return Vec::new();
        };
        let base = roaming.join("Microsoft");
        let mut names: Vec<String> = VERSIONS
            .iter()
            .chain(std::iter::once(&"14.0"))
            .filter_map(|v| {
                string(
                    &format!("Software\\Microsoft\\Office\\{v}\\Common\\General"),
                    "Signatures",
                )
            })
            .filter(|n| !n.contains(['\\', '/', ':']) && n != ".." && n != ".")
            .collect();
        names.push("Signatures".into());
        let mut out: Vec<PathBuf> = Vec::new();
        for n in names {
            let dir = base.join(n);
            if dir.is_dir() && !out.contains(&dir) {
                out.push(dir);
            }
        }
        out
    }

    pub(super) fn discover() -> Found {
        let mut keys = Vec::new();
        for v in VERSIONS {
            profile_keys(
                &format!("Software\\Microsoft\\Office\\{v}\\Outlook\\Profiles"),
                &mut keys,
            );
        }
        profile_keys(
            "Software\\Microsoft\\Windows NT\\CurrentVersion\\Windows Messaging Subsystem\\Profiles",
            &mut keys,
        );
        let mail = |name| {
            VERSIONS.iter().find_map(|v| {
                string(
                    &format!("Software\\Microsoft\\Office\\{v}\\Common\\MailSettings"),
                    name,
                )
            })
        };
        let (global_new, global_reply) = (mail("NewSignature"), mail("ReplySignature"));
        let accounts = collect_accounts(&keys, global_new.as_deref(), global_reply.as_deref());
        let mut signatures: Vec<FoundSignature> = Vec::new();
        for dir in signature_dirs() {
            let files = listing(&dir);
            let read = |rel: &str| {
                let path = dir.join(rel);
                let len = std::fs::metadata(&path).ok()?.len();
                (len <= MAX_SIGNATURE_BYTES)
                    .then(|| std::fs::read(&path).ok())
                    .flatten()
            };
            for s in signatures_in(&dir, &files, read) {
                if !signatures
                    .iter()
                    .any(|f| f.name.eq_ignore_ascii_case(&s.name))
                {
                    signatures.push(s);
                }
            }
        }
        let new_outlook_only = accounts.is_empty()
            && signatures.is_empty()
            && !classic_installed()
            && new_outlook_installed();
        Found {
            accounts,
            signatures,
            new_outlook_only,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(s: &str) -> RegValue {
        let mut b: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
        b.extend([0, 0]);
        RegValue::Binary(b)
    }

    fn sz(s: &str) -> RegValue {
        RegValue::Sz(s.into())
    }

    fn values(items: &[(&str, RegValue)]) -> Values {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    /// Outlook 2016 (16.0): strings as REG_BINARY UTF-16LE, ports and
    /// security as REG_DWORD.
    fn imap_2016() -> Values {
        values(&[
            ("Account Name", wide("ana@example.org")),
            ("Display Name", wide("Ana Silva")),
            ("Email", wide("ana@example.org")),
            ("IMAP Server", wide("imap.example.org")),
            ("IMAP Port", RegValue::Dword(993)),
            ("IMAP Use SSL", RegValue::Dword(1)),
            ("IMAP User", wide("ana")),
            ("IMAP Password", RegValue::Binary(vec![1, 2, 3, 4])),
            ("SMTP Server", wide("smtp.example.org")),
            ("SMTP Port", RegValue::Dword(587)),
            ("SMTP Secure Connection", RegValue::Dword(2)),
            ("SMTP Use Auth", RegValue::Dword(1)),
            ("New Signature", wide("Work")),
            ("Reply-Forward Signature", wide("Short")),
        ])
    }

    #[test]
    fn an_outlook_2016_imap_account_reads_fully() {
        let a = parse_account("Outlook", &imap_2016()).expect("account");
        assert_eq!(a.kind, Kind::Imap);
        assert_eq!(a.email, "ana@example.org");
        assert_eq!(a.display_name, "Ana Silva");
        assert_eq!(a.account_name, "ana@example.org");
        assert_eq!(a.profile, "Outlook");
        assert_eq!(
            a.incoming,
            Some(Server {
                host: "imap.example.org".into(),
                port: 993,
                security: Security::Tls
            })
        );
        assert_eq!(a.incoming_user, "ana");
        assert_eq!(
            a.smtp,
            Some(Server {
                host: "smtp.example.org".into(),
                port: 587,
                security: Security::StartTls
            })
        );
        assert_eq!(a.smtp_user, "");
        assert_eq!(a.new_signature.as_deref(), Some("Work"));
        assert_eq!(a.reply_signature.as_deref(), Some("Short"));
    }

    /// Outlook 2010 and earlier (Windows Messaging Subsystem): REG_SZ
    /// strings, `Use SSL` alone, a separate SMTP sign-in, no ports.
    #[test]
    fn an_older_layout_with_strings_and_defaults_reads() {
        let v = values(&[
            ("Account Name", sz("Home mail")),
            ("Display Name", sz("Rui")),
            ("Email", sz("rui@example.org")),
            ("IMAP Server", sz("mail.example.org")),
            ("IMAP Use SSL", RegValue::Dword(1)),
            ("SMTP Server", sz("mail.example.org")),
            ("SMTP Use SSL", RegValue::Dword(1)),
            ("SMTP Use Auth", RegValue::Dword(1)),
            ("SMTP User", sz("rui-send")),
        ]);
        let a = parse_account("Default", &v).expect("account");
        assert_eq!(a.account_name, "Home mail");
        assert_eq!(a.incoming.as_ref().map(|s| s.port), Some(993));
        assert_eq!(
            a.smtp.as_ref().map(|s| (s.port, s.security)),
            Some((465, Security::Tls))
        );
        assert_eq!(a.smtp_user, "rui-send");
        assert_eq!(a.incoming_user, "");
        assert_eq!(a.new_signature, None);
    }

    #[test]
    fn missing_values_take_the_usual_defaults() {
        let v = values(&[
            ("Email", wide("min@example.org")),
            ("IMAP Server", wide("imap.example.org")),
        ]);
        let a = parse_account("P", &v).expect("account");
        assert_eq!(a.display_name, "");
        assert_eq!(
            a.incoming.as_ref().map(|s| (s.port, s.security)),
            Some((143, Security::StartTls))
        );
        assert_eq!(a.smtp, None);
        // No address at all: not an account.
        assert_eq!(
            parse_account("P", &values(&[("IMAP Server", wide("x.example.org"))])),
            None
        );
        // An address book entry.
        assert_eq!(
            parse_account(
                "P",
                &values(&[
                    ("Account Name", wide("Outlook Address Book")),
                    ("LDAP Server", wide("ldap.example.org"))
                ])
            ),
            None
        );
    }

    #[test]
    fn exchange_and_pop_accounts_are_reported() {
        let ex = values(&[
            ("Account Name", wide("joana@example.org")),
            ("Display Name", wide("Joana")),
            ("Service UID", RegValue::Binary(vec![9; 16])),
        ]);
        let a = parse_account("Outlook", &ex).expect("exchange");
        assert_eq!(a.kind, Kind::Exchange);
        assert_eq!(a.email, "joana@example.org");
        assert_eq!((a.incoming, a.smtp), (None, None));

        let pop = values(&[
            ("Email", wide("old@example.org")),
            ("POP3 Server", wide("pop.example.org")),
            ("POP3 Port", RegValue::Dword(995)),
            ("POP3 User", wide("old")),
            ("SMTP Server", wide("smtp.example.org")),
            ("SMTP Port", RegValue::Dword(465)),
        ]);
        let p = parse_account("Outlook", &pop).expect("pop");
        assert_eq!(p.kind, Kind::Pop);
        assert_eq!(
            p.incoming
                .as_ref()
                .map(|s| (s.host.as_str(), s.port, s.security)),
            Some(("pop.example.org", 995, Security::Tls))
        );
        assert_eq!(p.incoming_user, "old");
        assert_eq!(p.smtp.as_ref().map(|s| s.security), Some(Security::Tls));
    }

    #[test]
    fn passwords_are_never_read() {
        let v = imap_2016();
        assert!(is_secret("IMAP Password"));
        assert!(is_secret("smtp password"));
        assert_eq!(text(&v, "IMAP Password"), None);
        assert_eq!(dword(&v, "IMAP Password"), None);
    }

    #[test]
    fn registry_text_decodes_utf16_and_narrow() {
        assert_eq!(binary_text(&[b'J', 0, 0xe3, 0, b'o', 0, 0, 0]), "J\u{e3}o");
        assert_eq!(binary_text(b"abc\0"), "abc");
        assert_eq!(binary_text(b"ab\0"), "ab");
        assert_eq!(binary_text(&[]), "");
        // Non-Latin UTF-16 with its NUL.
        let cjk: Vec<u8> = "\u{65e5}\u{672c}\0"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(binary_text(&cjk), "\u{65e5}\u{672c}");
        // A REG_DWORD stored as four binary bytes.
        let v = values(&[("IMAP Port", RegValue::Binary(993u32.to_le_bytes().to_vec()))]);
        assert_eq!(dword(&v, "IMAP Port"), Some(993));
        // Names match case aside.
        assert_eq!(dword(&v, "imap port"), Some(993));
    }

    #[test]
    fn profiles_merge_by_address_with_global_defaults() {
        let keys = vec![
            ("A".to_string(), imap_2016()),
            ("B".to_string(), imap_2016()),
            (
                "B".to_string(),
                values(&[
                    ("Email", wide("two@example.org")),
                    ("IMAP Server", wide("imap.example.org")),
                ]),
            ),
        ];
        let all = collect_accounts(&keys, Some("Global"), None);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].profile, "A");
        assert_eq!(all[0].new_signature.as_deref(), Some("Work"));
        assert_eq!(all[1].new_signature.as_deref(), Some("Global"));
        assert_eq!(all[1].reply_signature, None);
    }

    #[test]
    fn already_added_ignores_case() {
        let have = ["Ana@Example.org", "x@example.org"];
        assert!(already_added("ana@example.org", have));
        assert!(!already_added("rui@example.org", have));
    }

    #[test]
    fn signature_html_decodes_from_its_charset() {
        let cp = b"<html><head><meta http-equiv=Content-Type content=\"text/html; charset=windows-1252\"></head><body>Jo\xe3o \x80</body></html>";
        let (html, cs) = decode_html(cp);
        assert_eq!(cs, "windows-1252");
        assert!(html.contains("Jo\u{e3}o \u{20ac}"), "{html}");

        let (html, cs) = decode_html("<meta charset=\"utf-8\">Ol\u{e1}".as_bytes());
        assert_eq!(
            (html.as_str(), cs.as_str()),
            ("<meta charset=\"utf-8\">Ol\u{e1}", "utf-8")
        );

        let mut le = vec![0xff, 0xfe];
        le.extend("<p>Ol\u{e1}</p>".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(
            decode_html(&le),
            ("<p>Ol\u{e1}</p>".into(), "utf-16le".into())
        );

        // Undeclared and not UTF-8: Windows-1252.
        assert_eq!(decode_html(b"caf\xe9").0, "caf\u{e9}");
    }

    #[test]
    fn signatures_find_their_pictures_and_folders() {
        let dir = Path::new("C:\\Sig");
        let listing: Vec<String> = [
            "Work.htm",
            "Work.rtf",
            "Work.txt",
            "Work_files/image001.png",
            "Work_files/image002.jpg",
            "Work_files/filelist.xml",
            "Pessoal.htm",
            "Pessoal_ficheiros/image001.png",
            "Plain.htm",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let read = |rel: &str| -> Option<Vec<u8>> {
            Some(match rel {
                "Work.htm" => b"<meta charset=windows-1252><img src=\"Work_files/image001.png\"><v:imagedata o:href=\"cid:x\" src=\"Work_files/image002.jpg\"><img src=\"https://example.org/x.png\">".to_vec(),
                "Pessoal.htm" => b"<p>Ol\xe1</p>".to_vec(),
                "Plain.htm" => b"<p>Plain</p>".to_vec(),
                _ => return None,
            })
        };
        let found = signatures_in(dir, &listing, read);
        let names: Vec<&str> = found.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Pessoal", "Plain", "Work"]);
        let work = &found[2];
        assert_eq!(work.charset, "windows-1252");
        assert_eq!(
            work.images,
            [
                dir.join("Work_files\\image001.png"),
                dir.join("Work_files\\image002.jpg")
            ]
        );
        // No reference: the localised picture folder.
        assert_eq!(
            found[0].images,
            [dir.join("Pessoal_ficheiros\\image001.png")]
        );
        assert_eq!(found[0].html, "<p>Ol\u{e1}</p>");
        assert!(found[1].images.is_empty());
    }

    /// An Outlook-style signature (windows-1252, a layout table with a logo
    /// spanning two rows, coloured text, two pictures in its `_files`
    /// folder and one on the web) is saved with both local pictures, its
    /// table and colours; the web picture is left out, and counted.
    #[test]
    fn an_outlook_signature_keeps_its_pictures_and_layout() {
        use crate::mail::rich::images::tests::png;
        use crate::mail::rich::{Block, import};

        let dir = Path::new("C:\\Users\\Ana\\AppData\\Roaming\\Microsoft\\Signatures");
        let html: &[u8] = b"<html><head><meta http-equiv=Content-Type content=\"text/html; charset=windows-1252\"></head>\
<body lang=PT><table class=MsoTableGrid border=0 cellspacing=0 cellpadding=0 style='border-collapse:collapse;border:none'>\
<tr><td width=96 rowspan=2 valign=top style='padding:0cm'><p class=MsoNormal>\
<!--[if gte vml 1]><v:shape style='width:72pt;height:72pt'><v:imagedata src=\"Work_files/image001.png\" o:title=\"\"/></v:shape><![endif]-->\
<![if !vml]><img width=96 height=96 src=\"Work_files/image001.png\" alt=\"Logo\" v:shapes=\"Picture_1\"><![endif]></p></td>\
<td style='padding:0cm 0cm 0cm 8pt'><p class=MsoNormal><b><span style='color:#1F3864'>Jos\xe9 Example</span></b></p></td></tr>\
<tr><td style='padding:0cm 0cm 0cm 8pt'><p class=MsoNormal><span style='color:#7F7F7F'>Support lead</span></p>\
<p class=MsoNormal><img width=120 height=30 src=\"Work_files/image002.png\" alt=\"Partner\">\
<img width=88 height=31 src=\"https://example.org/badge.png\" alt=\"Badge\"></p></td></tr></table></body></html>";
        let logo = png(96, 96);
        let partner = png(120, 30);
        let listing: Vec<String> = [
            "Work.htm",
            "Work_files/image001.png",
            "Work_files/image002.png",
            "Work_files/filelist.xml",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let found = signatures_in(dir, &listing, |rel: &str| {
            (rel == "Work.htm").then(|| html.to_vec())
        });
        let [sig] = found.as_slice() else {
            panic!("{found:?}")
        };
        assert_eq!(sig.charset, "windows-1252");
        assert_eq!(sig.images.len(), 2);
        assert_eq!(web_pictures(&sig.html), 1);
        let read = |p: &Path| -> Option<Vec<u8>> {
            match p.file_name()?.to_str()? {
                "image001.png" => Some(logo.clone()),
                "image002.png" => Some(partner.clone()),
                _ => None,
            }
        };
        let (doc, images) = to_signature(sig, &read).expect("readable");
        assert_eq!(images.len(), 2, "both local pictures, not the web one");

        let db = crate::db::Db::in_memory().unwrap();
        let id = db.add_signature(&sig.name, &doc, &images).unwrap();
        let saved = db.signature(id).unwrap().unwrap();
        assert_eq!(saved.name, "Work");
        assert_eq!(saved.images.len(), 2);
        assert!(
            saved.body_text.contains("Jos\u{e9} Example"),
            "{}",
            saved.body_text
        );
        let back = import::strict(&saved.body_html).expect("stored form");
        let table = back
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Table(t) => Some(t),
                Block::Signature(s) => s.blocks.iter().find_map(|b| match b {
                    Block::Table(t) => Some(t),
                    _ => None,
                }),
                _ => None,
            })
            .expect("the layout table");
        assert!(table.borderless);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0][0].rowspan, 2, "the logo spans both rows");
        let pictures: Vec<&str> = table
            .rows
            .iter()
            .flatten()
            .flat_map(|c| &c.paras)
            .flat_map(|p| &p.runs)
            .filter_map(|r| r.image.as_ref().map(|i| i.cid.as_str()))
            .collect();
        assert_eq!(pictures.len(), 2, "{pictures:?}");
        assert!(pictures.iter().all(|c| saved.images.contains(c)));
        let coloured = table
            .rows
            .iter()
            .flatten()
            .flat_map(|c| &c.paras)
            .flat_map(|p| &p.runs)
            .filter(|r| r.style.color.is_some())
            .count();
        assert!(coloured >= 2, "the name's and the title's colours");
    }

    #[test]
    fn local_pictures_match_their_relative_address_only() {
        let sig = FoundSignature {
            name: "Work".into(),
            html: String::new(),
            charset: "utf-8".into(),
            images: vec![PathBuf::from("C:\\Sig\\My Sig_files\\image001.png")],
        };
        assert!(local_picture(&sig, "My%20Sig_files/image001.png").is_some());
        assert!(local_picture(&sig, "my sig_files\\IMAGE001.png").is_some());
        assert!(local_picture(&sig, "image001.png").is_some());
        assert!(local_picture(&sig, "Other_files/image001.png").is_none());
        assert!(local_picture(&sig, "../My Sig_files/image001.png").is_none());
        assert!(local_picture(&sig, "C:\\Sig\\My Sig_files\\image001.png").is_none());
        assert!(local_picture(&sig, "https://example.org/image001.png").is_none());
        assert_eq!(
            web_pictures("<IMG SRC = 'HTTPS://x/y.png'><img src=a.png><img src=\"http://x\">"),
            2
        );
    }

    #[test]
    fn references_skip_urls_and_parent_paths() {
        let html = "<img src='My%20Sig_files/image001.png'><img src=../evil.png><img src=\"cid:a\"><img SRC=a\\b.png>";
        assert_eq!(
            referenced_paths(html),
            ["My Sig_files/image001.png", "a/b.png"]
        );
    }

    #[test]
    fn found_is_empty_without_accounts_or_signatures() {
        assert!(Found::default().is_empty());
        let only_new = Found {
            new_outlook_only: true,
            ..Found::default()
        };
        assert!(only_new.is_empty());
    }
}
