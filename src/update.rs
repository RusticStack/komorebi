//! Updates from the project's GitHub releases.
//!
//! One HTTPS GET to the GitHub API for the latest release, started by the
//! user (Settings > About > Check for updates) or at most once a day when
//! automatic checks are on. Nothing is downloaded until the user presses
//! Download and install. The installer is accepted only when its SHA-256
//! matches the published `.sha256` and an Ed25519 signature over
//! [`signed_message`] (the version and that hash) verifies against
//! [`PUBLIC_KEY`], which is compiled in. Binding the version to the
//! signature stops an older, validly signed installer from being served as
//! a newer one. The request carries a fixed User-Agent and no identifiers;
//! `KOMOREBI_OFFLINE` refuses it (through `fetch`).
//!
//! Parsing is panic-free and capped: the release JSON is read by a small
//! non-recursive scanner that keeps only the fields below.

use crate::fetch::{self, FetchError, Options};
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The latest release of the project.
pub const LATEST_URL: &str = "https://api.github.com/repos/RusticStack/komorebi/releases/latest";
/// Every release page and download lives under this prefix.
const RELEASES: &str = "https://github.com/RusticStack/komorebi/releases/";
/// The Ed25519 public key that signs releases (examples/sign_update.rs).
/// Hex: 2c8d8b1771be23a6d09afb973095ec753bd5bc39a69feea28749f369e2a1a603.
pub const PUBLIC_KEY: [u8; 32] = [
    0x2c, 0x8d, 0x8b, 0x17, 0x71, 0xbe, 0x23, 0xa6, 0xd0, 0x9a, 0xfb, 0x97, 0x30, 0x95, 0xec, 0x75,
    0x3b, 0xd5, 0xbc, 0x39, 0xa6, 0x9f, 0xee, 0xa2, 0x87, 0x49, 0xf3, 0x69, 0xe2, 0xa1, 0xa6, 0x03,
];
/// The largest release JSON accepted.
pub const MAX_RELEASE_JSON: usize = 1024 * 1024;
/// The largest installer accepted.
pub const MAX_INSTALLER: usize = 64 * 1024 * 1024;
/// The largest `.sha256` or `.sig` file accepted.
const MAX_SMALL: usize = 4096;
/// The most assets read from one release.
const MAX_ASSETS: usize = 100;
/// The deepest JSON nesting accepted.
const MAX_DEPTH: usize = 32;
/// Automatic checks run at most this often (seconds).
pub const AUTO_INTERVAL_SECS: i64 = 24 * 60 * 60;
/// The settings keys: automatic checks ("1"/"0", default on) and the time
/// of the last automatic check (Unix seconds).
pub const SETTING_AUTO: &str = "update_auto";
pub const SETTING_LAST: &str = "update_last_check";
/// The installer's command line: silent, closes the running app first.
pub const INSTALLER_ARGS: [&str; 4] = [
    "/VERYSILENT",
    "/SUPPRESSMSGBOXES",
    "/CLOSEAPPLICATIONS",
    "/RESTARTAPPLICATIONS",
];

/// Why a check or an update failed. Rendered on the UI thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    /// `KOMOREBI_OFFLINE` is set.
    Offline,
    /// The request failed (network, TLS, timeout, HTTP status).
    Network,
    /// The release JSON is malformed, too large or not the project's.
    BadRelease,
    /// The release has no installer, `.sha256` or `.sig` for this version.
    MissingAsset,
    /// A download is over its cap.
    TooLarge,
    /// The installer's SHA-256 differs from the published one.
    HashMismatch,
    /// The signature does not verify against the compiled-in key.
    BadSignature,
    /// Writing or starting the installer failed.
    Io,
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            UpdateError::Offline => "offline",
            UpdateError::Network => "network error",
            UpdateError::BadRelease => "malformed release",
            UpdateError::MissingAsset => "release asset missing",
            UpdateError::TooLarge => "download too large",
            UpdateError::HashMismatch => "SHA-256 mismatch",
            UpdateError::BadSignature => "bad signature",
            UpdateError::Io => "file error",
        })
    }
}

impl std::error::Error for UpdateError {}

impl From<FetchError> for UpdateError {
    fn from(e: FetchError) -> Self {
        match e {
            FetchError::Offline => UpdateError::Offline,
            FetchError::TooLarge => UpdateError::TooLarge,
            _ => UpdateError::Network,
        }
    }
}

/// A release version `major.minor.patch`; pre-releases are not versions
/// here, so they are never offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// `0.4.0` or `v0.4.0`: three numbers of digits only, nothing else.
pub fn parse_version(s: &str) -> Option<Version> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let mut parts = s.split('.');
    let mut num = || -> Option<u32> {
        let p = parts.next()?;
        if p.is_empty() || p.len() > 9 || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        p.parse().ok()
    };
    let v = Version(num()?, num()?, num()?);
    parts.next().is_none().then_some(v)
}

/// The running app's version.
pub fn current() -> Version {
    parse_version(env!("CARGO_PKG_VERSION")).unwrap_or(Version(0, 0, 0))
}

/// A downloadable file of a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

/// A release that offers an installer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// The release page (release notes).
    pub notes_url: String,
    pub installer: Asset,
    pub sha256: Asset,
    pub sig: Asset,
}

/// The installer's file name for `v`, as `installer/komorebi.iss` names it.
pub fn installer_name(v: Version) -> String {
    format!("komorebi-{v}-setup.exe")
}

/// The bytes the release signature covers.
pub fn signed_message(v: Version, sha256_hex: &str) -> Vec<u8> {
    format!(
        "komorebi-update-v1\n{v}\n{}\n",
        sha256_hex.to_ascii_lowercase()
    )
    .into_bytes()
}

/// Lower-case hex.
pub fn to_hex(b: &[u8]) -> String {
    const D: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(b.len() * 2);
    for &x in b {
        s.push(D[usize::from(x >> 4)] as char);
        s.push(D[usize::from(x & 15)] as char);
    }
    s
}

/// Hex (either case) to bytes.
pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(2) {
        return None;
    }
    let val = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    b.chunks_exact(2)
        .map(|p| Some(val(p[0])? << 4 | val(p[1])?))
        .collect()
}

/// The SHA-256 of `data` in lower-case hex.
pub fn sha256_hex(data: &[u8]) -> String {
    to_hex(&Sha256::digest(data))
}

/// The newest release from the latest-release JSON, or `Ok(None)` when it
/// is not newer than `running`.
pub fn parse_release(json: &[u8], running: Version) -> Result<Option<Release>, UpdateError> {
    if json.len() > MAX_RELEASE_JSON {
        return Err(UpdateError::TooLarge);
    }
    let raw = scan(json).ok_or(UpdateError::BadRelease)?;
    if raw.draft || raw.prerelease {
        return Ok(None);
    }
    let tag = raw.tag.ok_or(UpdateError::BadRelease)?;
    let version = parse_version(&tag).ok_or(UpdateError::BadRelease)?;
    let notes_url = raw.html_url.ok_or(UpdateError::BadRelease)?;
    if !notes_url.starts_with(RELEASES) || notes_url.len() > 512 {
        return Err(UpdateError::BadRelease);
    }
    if version <= running {
        return Ok(None);
    }
    let name = installer_name(version);
    let find = |n: &str| -> Result<Asset, UpdateError> {
        let want = format!("{RELEASES}download/{tag}/{n}");
        raw.assets
            .iter()
            .find(|a| a.name == n)
            .filter(|a| a.url == want)
            .cloned()
            .ok_or(UpdateError::MissingAsset)
    };
    Ok(Some(Release {
        version,
        notes_url,
        installer: find(&name)?,
        sha256: find(&format!("{name}.sha256"))?,
        sig: find(&format!("{name}.sig"))?,
    }))
}

/// Accepts `installer` only when its SHA-256 matches `sha_file` (hex,
/// optionally followed by the file name) and `sig_file` (hex) is a valid
/// signature by `key` over [`signed_message`] for `version`.
pub fn verify(
    installer: &[u8],
    sha_file: &[u8],
    sig_file: &[u8],
    version: Version,
    key: &[u8; 32],
) -> Result<(), UpdateError> {
    if sha_file.len() > MAX_SMALL || sig_file.len() > MAX_SMALL {
        return Err(UpdateError::TooLarge);
    }
    let published = std::str::from_utf8(sha_file)
        .ok()
        .and_then(|s| s.split_whitespace().next())
        .filter(|h| h.len() == 64 && from_hex(h).is_some())
        .ok_or(UpdateError::HashMismatch)?
        .to_ascii_lowercase();
    let actual = sha256_hex(installer);
    if published != actual {
        return Err(UpdateError::HashMismatch);
    }
    let sig = std::str::from_utf8(sig_file)
        .ok()
        .map(str::trim)
        .and_then(from_hex)
        .and_then(|b| ed25519_compact::Signature::from_slice(&b).ok())
        .ok_or(UpdateError::BadSignature)?;
    ed25519_compact::PublicKey::new(*key)
        .verify(signed_message(version, &actual), &sig)
        .map_err(|_| UpdateError::BadSignature)
}

fn github_host(u: &url::Url) -> bool {
    matches!(
        u.host_str(),
        Some(
            "github.com" | "objects.githubusercontent.com" | "release-assets.githubusercontent.com"
        )
    )
}

fn user_agent() -> String {
    format!("Komorebi/{}", env!("CARGO_PKG_VERSION"))
}

/// Asks GitHub for the latest release: `Some` when it is newer than the
/// running app. Blocks; run it on a short-lived thread.
pub fn check() -> Result<Option<Release>, UpdateError> {
    let ua = user_agent();
    let body = fetch::get_with(
        LATEST_URL,
        &Options {
            accept: "application/vnd.github+json",
            max_body: MAX_RELEASE_JSON,
            total: Duration::from_secs(20),
            redirect_ok: &|u| u.host_str() == Some("api.github.com"),
            user_agent: &ua,
            progress: None,
        },
    )?;
    parse_release(&body, current())
}

fn download(
    url: &str,
    max: usize,
    total: Duration,
    progress: Option<&dyn Fn(u64, Option<u64>)>,
) -> Result<Vec<u8>, UpdateError> {
    if !url.starts_with(RELEASES) {
        return Err(UpdateError::BadRelease);
    }
    let ua = user_agent();
    Ok(fetch::get_with(
        url,
        &Options {
            accept: "application/octet-stream",
            max_body: max,
            total,
            redirect_ok: &github_host,
            user_agent: &ua,
            progress,
        },
    )?)
}

/// The least time between two progress reports of a download.
pub const PROGRESS_EVERY: Duration = Duration::from_millis(120);

/// Lets a progress report through at most every [`PROGRESS_EVERY`]: the
/// first, the last (everything received) and whatever is due in between.
#[derive(Debug, Default)]
pub struct Throttle {
    last: Option<std::time::Instant>,
}

impl Throttle {
    pub fn admit(&mut self, now: std::time::Instant, done: u64, total: Option<u64>) -> bool {
        let finished = total.is_some_and(|t| done >= t);
        let due = self
            .last
            .is_none_or(|l| now.saturating_duration_since(l) >= PROGRESS_EVERY);
        if finished || due {
            self.last = Some(now);
        }
        finished || due
    }
}

/// The fraction `done` of `total`, 0 to 1; `None` without a usable total.
pub fn fraction(done: u64, total: Option<u64>) -> Option<f32> {
    total
        .filter(|&t| t > 0)
        .map(|t| (done.min(t) as f64 / t as f64) as f32)
}

/// Downloads and verifies `rel`'s installer, then writes it to a new folder
/// under `temp` and returns its path. `progress` hears the installer's
/// bytes received and its announced size, at most every
/// [`PROGRESS_EVERY`]. Blocks; run it on a short-lived thread.
pub fn fetch_installer(
    rel: &Release,
    temp: &Path,
    progress: &dyn Fn(u64, Option<u64>),
) -> Result<PathBuf, UpdateError> {
    let small = Duration::from_secs(20);
    let sha = download(&rel.sha256.url, MAX_SMALL, small, None)?;
    let sig = download(&rel.sig.url, MAX_SMALL, small, None)?;
    let throttle = std::cell::RefCell::new(Throttle::default());
    let report = |done: u64, total: Option<u64>| {
        if throttle
            .borrow_mut()
            .admit(std::time::Instant::now(), done, total)
        {
            progress(done, total);
        }
    };
    let exe = download(
        &rel.installer.url,
        MAX_INSTALLER,
        Duration::from_secs(600),
        Some(&report),
    )?;
    verify(&exe, &sha, &sig, rel.version, &PUBLIC_KEY)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = temp.join(format!("komorebi-update-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&dir).map_err(|_| UpdateError::Io)?;
    let path = dir.join(installer_name(rel.version));
    std::fs::write(&path, &exe).map_err(|_| UpdateError::Io)?;
    Ok(path)
}

/// Starts the verified installer; the caller saves its state and quits.
pub fn run_installer(path: &Path) -> Result<(), UpdateError> {
    std::process::Command::new(path)
        .args(INSTALLER_ARGS)
        .spawn()
        .map(drop)
        .map_err(|_| UpdateError::Io)
}

/// Whether an automatic check is due: on, and none in the last day.
pub fn auto_due(auto: Option<&str>, last: Option<&str>, now: i64) -> bool {
    if auto == Some("0") {
        return false;
    }
    let last = last.and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
    !(0..AUTO_INTERVAL_SECS).contains(&(now - last))
}

/// Where Settings > About stands. Moves only through the methods below, so
/// a check never runs twice at once and an install never starts without a
/// verified download.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Phase {
    /// Nothing asked yet this session.
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading(Release),
    /// A check or a download failed; the release, if one was found, can be
    /// tried again.
    Failed(UpdateError, Option<Release>),
    /// The verified installer was started; the app is quitting.
    Installing,
}

impl Phase {
    /// The number the UI shows by: 0 idle, 1 checking, 2 up to date, 3
    /// available, 4 downloading, 5 failed, 6 installing.
    pub fn code(&self) -> i32 {
        match self {
            Phase::Idle => 0,
            Phase::Checking => 1,
            Phase::UpToDate => 2,
            Phase::Available(_) => 3,
            Phase::Downloading(_) => 4,
            Phase::Failed(..) => 5,
            Phase::Installing => 6,
        }
    }

    /// The release found, in the phases that have one.
    pub fn release(&self) -> Option<&Release> {
        match self {
            Phase::Available(r) | Phase::Downloading(r) | Phase::Failed(_, Some(r)) => Some(r),
            _ => None,
        }
    }

    /// Whether a network job runs (or the app is quitting).
    pub fn busy(&self) -> bool {
        matches!(
            self,
            Phase::Checking | Phase::Downloading(_) | Phase::Installing
        )
    }

    /// Check for updates pressed (or due): false while busy.
    pub fn start_check(&mut self) -> bool {
        if self.busy() {
            return false;
        }
        *self = Phase::Checking;
        true
    }

    /// The check answered; ignored unless one runs.
    pub fn checked(&mut self, result: Result<Option<Release>, UpdateError>) {
        if *self != Phase::Checking {
            return;
        }
        *self = match result {
            Ok(Some(r)) => Phase::Available(r),
            Ok(None) => Phase::UpToDate,
            Err(e) => Phase::Failed(e, None),
        };
    }

    /// Download and install pressed: the release to fetch, if there is one
    /// and nothing runs.
    pub fn start_download(&mut self) -> Option<Release> {
        let r = match self {
            Phase::Available(r) | Phase::Failed(_, Some(r)) => r.clone(),
            _ => return None,
        };
        *self = Phase::Downloading(r.clone());
        Some(r)
    }

    /// The download answered: true when the verified installer may run.
    pub fn downloaded(&mut self, result: Result<(), UpdateError>) -> bool {
        let Phase::Downloading(r) = self else {
            return false;
        };
        match result {
            Ok(()) => {
                *self = Phase::Installing;
                true
            }
            Err(e) => {
                *self = Phase::Failed(e, Some(r.clone()));
                false
            }
        }
    }
}

/// The number the UI shows an error by: 0 offline, 1 network, 2 malformed
/// or incomplete release, 3 too large, 4 hash or signature, 5 file.
pub fn error_code(e: &UpdateError) -> i32 {
    match e {
        UpdateError::Offline => 0,
        UpdateError::Network => 1,
        UpdateError::BadRelease | UpdateError::MissingAsset => 2,
        UpdateError::TooLarge => 3,
        UpdateError::HashMismatch | UpdateError::BadSignature => 4,
        UpdateError::Io => 5,
    }
}

/// The day number (UTC) of Unix time `now`: "Later" hides the bar until it
/// changes.
pub fn day_of(now: i64) -> i64 {
    now.div_euclid(24 * 60 * 60)
}

// --- The release JSON scanner ---

#[derive(Default)]
struct Raw {
    tag: Option<String>,
    html_url: Option<String>,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Default)]
struct PartAsset {
    name: Option<String>,
    url: Option<String>,
}

struct Frame {
    obj: bool,
    /// The current key, in an object.
    key: Option<String>,
}

#[derive(Clone, Copy)]
enum Expect {
    Value,
    ValueOrEnd,
    KeyOrEnd,
    Key,
    Colon,
    CommaOrEnd,
}

/// Sets `slot` once; a repeated field fails the scan.
fn once(slot: &mut Option<String>, v: String) -> Option<()> {
    if slot.is_some() {
        return None;
    }
    *slot = Some(v);
    Some(())
}

fn scan(b: &[u8]) -> Option<Raw> {
    let mut raw = Raw::default();
    let mut asset: Option<PartAsset> = None;
    let mut stack: Vec<Frame> = Vec::new();
    let mut expect = Expect::Value;
    let mut i = 0usize;
    let mut started = false;
    loop {
        while b.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
            i += 1;
        }
        if started && stack.is_empty() {
            return (i == b.len()).then_some(raw);
        }
        let c = *b.get(i)?;
        // Where the scanner is: the top object, or an asset object.
        let top_key = |stack: &[Frame]| stack.first().and_then(|f| f.key.clone());
        match expect {
            Expect::KeyOrEnd | Expect::ValueOrEnd | Expect::CommaOrEnd
                if c == b'}' || c == b']' =>
            {
                let f = stack.pop()?;
                if f.obj != (c == b'}') {
                    return None;
                }
                i += 1;
                if f.obj
                    && stack.len() == 2
                    && let Some(PartAsset {
                        name: Some(name),
                        url: Some(url),
                    }) = asset.take()
                {
                    if raw.assets.len() >= MAX_ASSETS {
                        return None;
                    }
                    raw.assets.push(Asset { name, url });
                }
                expect = Expect::CommaOrEnd;
            }
            Expect::CommaOrEnd => {
                if c != b',' {
                    return None;
                }
                i += 1;
                expect = if stack.last()?.obj {
                    Expect::Key
                } else {
                    Expect::Value
                };
            }
            Expect::KeyOrEnd | Expect::Key => {
                if c != b'"' {
                    return None;
                }
                let (s, n) = string(b, i)?;
                i = n;
                stack.last_mut()?.key = Some(s);
                expect = Expect::Colon;
            }
            Expect::Colon => {
                if c != b':' {
                    return None;
                }
                i += 1;
                expect = Expect::Value;
            }
            Expect::Value | Expect::ValueOrEnd => {
                if !started && c != b'{' {
                    return None;
                }
                started = true;
                let depth = stack.len();
                let key = stack.last().and_then(|f| f.key.clone());
                let in_assets = depth >= 2 && top_key(&stack).as_deref() == Some("assets");
                match c {
                    b'{' | b'[' => {
                        if depth >= MAX_DEPTH {
                            return None;
                        }
                        let obj = c == b'{';
                        if obj && depth == 2 && in_assets && !stack[1].obj {
                            asset = Some(PartAsset::default());
                        }
                        stack.push(Frame { obj, key: None });
                        i += 1;
                        expect = if obj {
                            Expect::KeyOrEnd
                        } else {
                            Expect::ValueOrEnd
                        };
                        continue;
                    }
                    b'"' => {
                        let (s, n) = string(b, i)?;
                        i = n;
                        match (depth, key.as_deref()) {
                            (1, Some("tag_name")) => once(&mut raw.tag, s)?,
                            (1, Some("html_url")) => once(&mut raw.html_url, s)?,
                            (3, Some(k)) if in_assets => {
                                if let Some(a) = asset.as_mut() {
                                    match k {
                                        "name" => once(&mut a.name, s)?,
                                        "browser_download_url" => once(&mut a.url, s)?,
                                        _ => {}
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {
                        let start = i;
                        while b
                            .get(i)
                            .is_some_and(|c| c.is_ascii_alphanumeric() || b"+-.".contains(c))
                        {
                            i += 1;
                        }
                        let word = b.get(start..i)?;
                        let truthy = match word {
                            b"true" => true,
                            b"false" | b"null" => false,
                            w if !w.is_empty()
                                && w.iter().all(|c| c.is_ascii_digit() || b"+-.eE".contains(c)) =>
                            {
                                false
                            }
                            _ => return None,
                        };
                        if depth == 1 && truthy {
                            match key.as_deref() {
                                Some("draft") => raw.draft = true,
                                Some("prerelease") => raw.prerelease = true,
                                _ => {}
                            }
                        }
                    }
                }
                expect = Expect::CommaOrEnd;
            }
        }
    }
}

/// The JSON string starting at `b[i] == '"'` and the index after it.
fn string(b: &[u8], mut i: usize) -> Option<(String, usize)> {
    i += 1;
    let mut out: Vec<u8> = Vec::new();
    loop {
        let c = *b.get(i)?;
        i += 1;
        match c {
            b'"' => return Some((String::from_utf8(out).ok()?, i)),
            b'\\' => {
                let e = *b.get(i)?;
                i += 1;
                let ch = match e {
                    b'"' => '"',
                    b'\\' => '\\',
                    b'/' => '/',
                    b'b' => '\u{8}',
                    b'f' => '\u{c}',
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    b'u' => {
                        let hex4 = |i: usize| -> Option<u32> {
                            let s = std::str::from_utf8(b.get(i..i + 4)?).ok()?;
                            u32::from_str_radix(s, 16).ok()
                        };
                        let hi = hex4(i)?;
                        i += 4;
                        if (0xD800..0xDC00).contains(&hi) {
                            if b.get(i..i + 2)? != b"\\u" {
                                return None;
                            }
                            let lo = hex4(i + 2)?;
                            if !(0xDC00..0xE000).contains(&lo) {
                                return None;
                            }
                            i += 6;
                            char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00))?
                        } else {
                            char::from_u32(hi)?
                        }
                    }
                    _ => return None,
                };
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
            0..=0x1f => return None,
            _ => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_compact::{KeyPair, Seed};

    fn json(tag: &str, assets: &[(&str, &str)]) -> String {
        let list: Vec<String> = assets
            .iter()
            .map(|(n, u)| format!(r#"{{"id":1,"name":"{n}","size":10,"uploader":{{"login":"x"}},"browser_download_url":"{u}"}}"#))
            .collect();
        format!(
            r#"{{"url":"https://api.github.com/x","html_url":"https://github.com/RusticStack/komorebi/releases/tag/{tag}","tag_name":"{tag}","draft":false,"prerelease":false,"author":{{"login":"a","id":2}},"assets":[{}],"body":"Notes \"quoted\" \u00e9 \ud83d\ude00"}}"#,
            list.join(",")
        )
    }

    fn full(tag: &str, v: &str) -> String {
        let base = format!("https://github.com/RusticStack/komorebi/releases/download/{tag}/");
        let n = format!("komorebi-{v}-setup.exe");
        json(
            tag,
            &[
                (&n, &format!("{base}{n}")),
                (&format!("{n}.sha256"), &format!("{base}{n}.sha256")),
                (&format!("{n}.sig"), &format!("{base}{n}.sig")),
            ],
        )
    }

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.4.0"), Some(Version(0, 4, 0)));
        assert_eq!(parse_version("1.10.2"), Some(Version(1, 10, 2)));
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert!(Version(1, 0, 0) > Version(0, 99, 99));
        for bad in [
            "",
            "v",
            "1.2",
            "1.2.3.4",
            "1.2.3-rc1",
            "1.2.x",
            " 1.2.3",
            "1..3",
            "+1.2.3",
        ] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
        assert_eq!(current(), parse_version(env!("CARGO_PKG_VERSION")).unwrap());
    }

    #[test]
    fn parses_a_newer_release() {
        let r = parse_release(full("v9.1.0", "9.1.0").as_bytes(), Version(0, 3, 0))
            .unwrap()
            .unwrap();
        assert_eq!(r.version, Version(9, 1, 0));
        assert_eq!(r.installer.name, "komorebi-9.1.0-setup.exe");
        assert!(r.sig.url.ends_with("/v9.1.0/komorebi-9.1.0-setup.exe.sig"));
        assert!(r.notes_url.starts_with(RELEASES));
    }

    #[test]
    fn same_or_older_is_not_offered() {
        let j = full("v0.3.0", "0.3.0");
        assert_eq!(parse_release(j.as_bytes(), Version(0, 3, 0)), Ok(None));
        assert_eq!(parse_release(j.as_bytes(), Version(0, 4, 0)), Ok(None));
        let pre = full("v9.0.0", "9.0.0").replace(r#""prerelease":false"#, r#""prerelease":true"#);
        assert_eq!(parse_release(pre.as_bytes(), Version(0, 3, 0)), Ok(None));
    }

    #[test]
    fn rejects_tampered_releases() {
        let old = Version(0, 3, 0);
        // An asset hosted elsewhere, or under another tag.
        let j = full("v9.0.0", "9.0.0").replace(
            "https://github.com/RusticStack/komorebi/releases/download/v9.0.0/komorebi-9.0.0-setup.exe\"",
            "https://evil.example/komorebi-9.0.0-setup.exe\"",
        );
        assert_eq!(
            parse_release(j.as_bytes(), old),
            Err(UpdateError::MissingAsset)
        );
        let j = full("v9.0.0", "9.0.0").replace("/download/v9.0.0/", "/download/v1.0.0/");
        assert_eq!(
            parse_release(j.as_bytes(), old),
            Err(UpdateError::MissingAsset)
        );
        // Missing assets.
        let j = json("v9.0.0", &[]);
        assert_eq!(
            parse_release(j.as_bytes(), old),
            Err(UpdateError::MissingAsset)
        );
        // A notes page elsewhere, a bad tag, a repeated field.
        let j = full("v9.0.0", "9.0.0").replace(
            "https://github.com/RusticStack/komorebi/releases/tag",
            "https://evil.example",
        );
        assert_eq!(
            parse_release(j.as_bytes(), old),
            Err(UpdateError::BadRelease)
        );
        let j = full("v9.0.0-rc1", "9.0.0");
        assert_eq!(
            parse_release(j.as_bytes(), old),
            Err(UpdateError::BadRelease)
        );
        let j = full("v9.0.0", "9.0.0").replace(r#""draft":false"#, r#""tag_name":"v99.0.0""#);
        assert_eq!(
            parse_release(j.as_bytes(), old),
            Err(UpdateError::BadRelease)
        );
        // Malformed JSON.
        let good = full("v9.0.0", "9.0.0");
        for bad in [
            &good[..good.len() - 1],
            "[]",
            "",
            "{\"tag_name\":\"v9.0.0\"} x",
            "{\"a\":[}",
            "{\"a\":tru}",
            "{\"a\":\"\\x\"}",
            "{\"a\":\"\\ud800\"}",
        ] {
            assert_eq!(
                parse_release(bad.as_bytes(), old),
                Err(UpdateError::BadRelease),
                "{bad}"
            );
        }
        // Too deep, too large, too many assets.
        let deep = format!("{{\"a\":{}{}}}", "[".repeat(100), "]".repeat(100));
        assert_eq!(
            parse_release(deep.as_bytes(), old),
            Err(UpdateError::BadRelease)
        );
        let huge = format!("{{\"a\":\"{}\"}}", "x".repeat(MAX_RELEASE_JSON));
        assert_eq!(
            parse_release(huge.as_bytes(), old),
            Err(UpdateError::TooLarge)
        );
        let many: Vec<(&str, &str)> = vec![("a", "b"); MAX_ASSETS + 1];
        assert_eq!(
            parse_release(json("v9.0.0", &many).as_bytes(), old),
            Err(UpdateError::BadRelease)
        );
    }

    fn signed(kp: &KeyPair, v: Version, data: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let sha = sha256_hex(data);
        let sig = kp.sk.sign(signed_message(v, &sha), None);
        (
            format!("{sha}  komorebi-{v}-setup.exe\n").into_bytes(),
            to_hex(sig.as_ref()).into_bytes(),
        )
    }

    #[test]
    fn verification() {
        let kp = KeyPair::from_seed(Seed::new([7; 32]));
        let pk: [u8; 32] = *kp.pk;
        let v = Version(9, 0, 0);
        let exe = b"installer bytes".to_vec();
        let (sha, sig) = signed(&kp, v, &exe);
        assert_eq!(verify(&exe, &sha, &sig, v, &pk), Ok(()));
        // A changed installer.
        let mut evil = exe.clone();
        evil[0] ^= 1;
        assert_eq!(
            verify(&evil, &sha, &sig, v, &pk),
            Err(UpdateError::HashMismatch)
        );
        // A matching hash but a signature by another key.
        let other = KeyPair::from_seed(Seed::new([8; 32]));
        let (sha2, sig2) = signed(&other, v, &evil);
        assert_eq!(
            verify(&evil, &sha2, &sig2, v, &pk),
            Err(UpdateError::BadSignature)
        );
        // An old, validly signed installer served as a newer version.
        let (old_sha, old_sig) = signed(&kp, Version(0, 1, 0), &exe);
        assert_eq!(
            verify(&exe, &old_sha, &old_sig, v, &pk),
            Err(UpdateError::BadSignature)
        );
        // Garbage files.
        assert_eq!(
            verify(&exe, b"zz", &sig, v, &pk),
            Err(UpdateError::HashMismatch)
        );
        assert_eq!(
            verify(&exe, &sha, b"00", v, &pk),
            Err(UpdateError::BadSignature)
        );
        assert_eq!(
            verify(&exe, &sha, &[b'0'; 5000], v, &pk),
            Err(UpdateError::TooLarge)
        );
    }

    #[test]
    fn the_shipped_key_is_set() {
        assert_ne!(PUBLIC_KEY, [0; 32]);
    }

    #[test]
    fn hex_and_hash() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(from_hex("0aFf"), Some(vec![10, 255]));
        assert_eq!(from_hex("0g"), None);
        assert_eq!(from_hex("0"), None);
    }

    fn release(v: Version) -> Release {
        let a = |n: &str| Asset {
            name: n.into(),
            url: format!("{RELEASES}download/v{v}/{n}"),
        };
        Release {
            version: v,
            notes_url: format!("{RELEASES}tag/v{v}"),
            installer: a("i"),
            sha256: a("s"),
            sig: a("g"),
        }
    }

    #[test]
    fn phases() {
        let r = release(Version(9, 0, 0));
        let mut p = Phase::default();
        assert_eq!(p.code(), 0);
        // Nothing to download before a check found something.
        assert_eq!(p.start_download(), None);
        assert!(p.start_check());
        assert!(!p.start_check(), "one check at a time");
        assert_eq!(p.code(), 1);
        p.checked(Ok(None));
        assert_eq!(p, Phase::UpToDate);
        // A late answer when no check runs changes nothing.
        p.checked(Ok(Some(r.clone())));
        assert_eq!(p, Phase::UpToDate);
        assert!(p.start_check());
        p.checked(Err(UpdateError::Network));
        assert_eq!(p, Phase::Failed(UpdateError::Network, None));
        assert_eq!(p.start_download(), None);
        assert!(p.start_check());
        p.checked(Ok(Some(r.clone())));
        assert_eq!(p.code(), 3);
        assert_eq!(p.release(), Some(&r));
        assert_eq!(p.start_download(), Some(r.clone()));
        assert!(p.busy());
        assert!(!p.start_check(), "no check while downloading");
        assert_eq!(p.start_download(), None, "one download at a time");
        // A failed verify keeps the release, to try again.
        assert!(!p.downloaded(Err(UpdateError::BadSignature)));
        assert_eq!(p.code(), 5);
        assert_eq!(p.release(), Some(&r));
        assert_eq!(p.start_download(), Some(r));
        assert!(p.downloaded(Ok(())));
        assert_eq!(p, Phase::Installing);
        assert!(!p.start_check());
        assert!(!p.downloaded(Ok(())), "installs once");
    }

    #[test]
    fn error_codes_and_days() {
        assert_eq!(error_code(&UpdateError::Offline), 0);
        assert_eq!(error_code(&UpdateError::MissingAsset), 2);
        assert_eq!(error_code(&UpdateError::BadSignature), 4);
        assert_eq!(error_code(&UpdateError::Io), 5);
        assert_eq!(day_of(0), 0);
        assert_eq!(day_of(86_399), 0);
        assert_eq!(day_of(86_400), 1);
        assert_eq!(day_of(-1), -1);
    }

    #[test]
    fn daily_auto_check() {
        assert!(auto_due(None, None, 100_000));
        assert!(!auto_due(Some("0"), None, 100_000));
        assert!(!auto_due(Some("1"), Some("90000"), 100_000));
        assert!(auto_due(Some("1"), Some("10000"), 100_000));
        // A clock set back does not stop checks forever.
        assert!(auto_due(None, Some("999999999"), 100_000));
    }

    #[test]
    fn progress_is_throttled_but_first_and_last_always_pass() {
        use std::time::Instant;
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut t = Throttle::default();
        assert!(t.admit(ms(0), 0, Some(100)));
        assert!(!t.admit(ms(10), 10, Some(100)));
        assert!(!t.admit(ms(119), 20, Some(100)));
        assert!(t.admit(ms(120), 30, Some(100)));
        assert!(!t.admit(ms(130), 40, Some(100)));
        // The last report passes at once, so the bar reaches the end.
        assert!(t.admit(ms(131), 100, Some(100)));
        // Without a total only time lets a report through.
        let mut t = Throttle::default();
        assert!(t.admit(ms(0), 5, None));
        assert!(!t.admit(ms(50), 9, None));
        assert!(t.admit(ms(500), 9, None));
    }

    #[test]
    fn fraction_needs_a_total_and_never_exceeds_one() {
        assert_eq!(fraction(0, None), None);
        assert_eq!(fraction(5, Some(0)), None);
        assert_eq!(fraction(0, Some(10)), Some(0.0));
        assert_eq!(fraction(5, Some(10)), Some(0.5));
        assert_eq!(fraction(20, Some(10)), Some(1.0));
    }
}
