//! Account icons: the picture beside an account's name. In order: an image
//! the user chose, else the icon of the mail domain's own website, else the
//! bundled icon of the account's provider, else nothing (the line looks as
//! it always did).
//!
//! The domain's icon is asked of that domain only, never of an icon
//! service: `https://<domain>/` is read for `<link rel="icon">` and
//! `apple-touch-icon`, then `/favicon.ico`. HTTPS only, no cookies, no
//! Referer, the generic User-Agent of [`crate::fetch`], short timeouts, at
//! most three redirects (each to the same domain), a size cap on each
//! response, raster formats only (PNG, ICO, JPEG, GIF, BMP, WebP; SVG is
//! refused). It is decoded on the thread that fetched it, scaled to
//! [`PIXELS`] (a 20 px slot at up to 200%), and kept in the cache's settings
//! as a PNG. It is fetched when an account is added and again after
//! [`REFRESH`], never per launch, never under `KOMOREBI_OFFLINE`, never for
//! a domain a bundled provider owns (its icon is the provider's), and never
//! once the switch ([`set_enabled`]) is off, which also drops what was
//! fetched.

use std::time::Duration;

use url::Url;

use crate::db::{Account, Db};
use crate::fetch::{self, FetchError, Options};
use crate::mail::render::img;
use crate::providers::{self, PRESETS};

/// The slot an icon is shown in, in pixels at 100%.
pub const SLOT: u32 = 20;
/// The pixels kept: the slot at 200%.
pub const PIXELS: u32 = SLOT * 2;
/// How long a fetched icon (or a domain found without one) is kept before
/// it is asked for again.
pub const REFRESH: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// The home page read for its icon links, at most.
pub const MAX_PAGE: usize = 256 * 1024;
/// An icon file, at most.
pub const MAX_ICON: usize = 256 * 1024;
/// One request, redirects included.
pub const TIMEOUT: Duration = Duration::from_secs(8);
/// Icon files tried from the page's links, before `/favicon.ico`.
const MAX_LINKS: usize = 3;
/// The largest icon decoded (an ICO entry, a BMP).
const MAX_SIDE: u32 = 1024;

/// The switch: `0` turns domain icons off.
const KEY_ON: &str = "account_icons";

fn key(account: i64) -> String {
    format!("account_icon.{account}")
}

/// An icon ready to show: a PNG of [`PIXELS`] square.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Icon {
    pub png: Vec<u8>,
    /// Dark marks on a clear ground: on the dark theme it needs a light
    /// backing (BRANDING.md: a neutral plate, only then).
    pub plate: bool,
}

/// Where a kept icon came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The user chose it.
    User,
    /// The mail domain's website.
    Domain,
    /// The domain was asked and had none: nothing until [`REFRESH`].
    Missing,
}

/// What the cache keeps for an account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    pub origin: Origin,
    /// Unix seconds it was fetched or chosen.
    pub at: i64,
    pub icon: Option<Icon>,
}

/// What an account's line shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shown {
    Image(Icon),
    /// The bundled icon of this provider preset (`Preset::id`).
    Provider(&'static str),
    None,
}

// ---------------------------------------------------------------- storage

/// Whether domain icons are fetched (on unless switched off).
pub fn enabled(db: &Db) -> bool {
    db.setting(KEY_ON).ok().flatten().as_deref() != Some("0")
}

/// Turn domain icons on or off. Off drops every fetched icon (the images
/// the user chose stay: they never came from the network).
pub fn set_enabled(db: &Db, on: bool) -> anyhow::Result<()> {
    db.set_setting(KEY_ON, if on { "1" } else { "0" })?;
    if !on {
        for a in db.accounts()? {
            if load(db, a.id).is_some_and(|k| k.origin != Origin::User) {
                db.delete_setting(&key(a.id))?;
            }
        }
    }
    Ok(())
}

/// What the cache keeps for `account`.
pub fn load(db: &Db, account: i64) -> Option<Kept> {
    let value = db.setting(&key(account)).ok()??;
    let mut parts = value.splitn(4, ';');
    let origin = match parts.next()? {
        "user" => Origin::User,
        "domain" => Origin::Domain,
        "missing" => Origin::Missing,
        _ => return None,
    };
    let at = parts.next()?.parse().ok()?;
    let plate = parts.next()? == "1";
    let data = parts.next().unwrap_or("");
    let icon = match origin {
        Origin::Missing => None,
        _ => Some(Icon {
            png: base64_decode(data)?,
            plate,
        }),
    };
    Some(Kept { origin, at, icon })
}

fn save(db: &Db, account: i64, kept: &Kept) -> anyhow::Result<()> {
    let origin = match kept.origin {
        Origin::User => "user",
        Origin::Domain => "domain",
        Origin::Missing => "missing",
    };
    let (plate, data) = kept
        .icon
        .as_ref()
        .map_or((false, String::new()), |i| (i.plate, base64_encode(&i.png)));
    db.set_setting(
        &key(account),
        &format!("{origin};{};{};{data}", kept.at, u8::from(plate)),
    )?;
    Ok(())
}

/// Keep an image the user chose (any raster file), made into an icon.
pub fn choose(db: &Db, account: i64, bytes: &[u8], now: i64) -> Result<(), Refusal> {
    let icon = decode(bytes)?;
    save(
        db,
        account,
        &Kept {
            origin: Origin::User,
            at: now,
            icon: Some(icon),
        },
    )
    .map_err(|_| Refusal::Undecodable)
}

/// Remove the image the user chose: the domain's or the provider's icon
/// shows again (fetched again when it is due).
pub fn remove(db: &Db, account: i64) -> anyhow::Result<()> {
    db.delete_setting(&key(account))?;
    Ok(())
}

/// Forget an account's icon (the account is removed).
pub fn forget(db: &Db, account: i64) -> anyhow::Result<()> {
    remove(db, account)
}

/// What `account`'s line shows, in order: the user's image, the domain's
/// icon (while the switch is on), the provider's, nothing.
pub fn shown(db: &Db, account: &Account) -> Shown {
    let kept = load(db, account.id);
    if let Some(Kept {
        origin: Origin::User,
        icon: Some(icon),
        ..
    }) = &kept
    {
        return Shown::Image(icon.clone());
    }
    if enabled(db)
        && let Some(Kept {
            origin: Origin::Domain,
            icon: Some(icon),
            ..
        }) = kept
    {
        return Shown::Image(icon);
    }
    provider(account).map_or(Shown::None, Shown::Provider)
}

/// The bundled provider an account is on: by its address's domain, else
/// by its IMAP server's host.
pub fn provider(account: &Account) -> Option<&'static str> {
    let by_domain = providers::guess(&account.email);
    let host = account
        .imap
        .host
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    by_domain
        .or_else(|| {
            PRESETS
                .iter()
                .position(|p| !host.is_empty() && p.imap.0.eq_ignore_ascii_case(&host))
        })
        .and_then(|i| PRESETS.get(i))
        .map(|p| p.id)
}

/// The mail domain an account's icon is asked of, if it is one to ask:
/// a public name (no IP, no single label), not a bundled provider's.
pub fn domain_to_ask(account: &Account) -> Option<String> {
    if providers::guess(&account.email).is_some() {
        return None;
    }
    let (_, domain) = account.email.trim().rsplit_once('@')?;
    let url = Url::parse(&format!("https://{}/", domain.trim())).ok()?;
    let host = match url.host()? {
        url::Host::Domain(d) => d.to_ascii_lowercase(),
        _ => return None,
    };
    (host.contains('.') && !host.ends_with(".invalid") && !host.ends_with(".localhost"))
        .then_some(host)
}

/// Whether `account`'s domain should be asked now (the switch is on, it is
/// not offline, no image was chosen, none was asked within [`REFRESH`]).
pub fn due(db: &Db, account: &Account, now: i64) -> bool {
    if !enabled(db) || crate::dev_var("KOMOREBI_OFFLINE").is_some() {
        return false;
    }
    if domain_to_ask(account).is_none() {
        return false;
    }
    match load(db, account.id) {
        Some(Kept {
            origin: Origin::User,
            ..
        }) => false,
        Some(k) => now.saturating_sub(k.at) >= REFRESH.as_secs() as i64,
        None => true,
    }
}

/// Keep what a fetch found for `account` (`None`: the domain had none).
/// Nothing is kept once the switch went off meanwhile, nor over an image
/// the user chose meanwhile.
pub fn keep(db: &Db, account: i64, found: Option<Icon>, now: i64) -> anyhow::Result<()> {
    if !enabled(db) || load(db, account).is_some_and(|k| k.origin == Origin::User) {
        return Ok(());
    }
    let origin = if found.is_some() {
        Origin::Domain
    } else {
        Origin::Missing
    };
    save(
        db,
        account,
        &Kept {
            origin,
            at: now,
            icon: found,
        },
    )
}

// ---------------------------------------------------------------- fetch

/// A GET as [`fetch::get_with`] makes it (tests: a local server).
pub type Get<'a> = &'a dyn Fn(&str, &Options) -> Result<Vec<u8>, FetchError>;

/// The production GET.
pub fn get(url: &str, opts: &Options) -> Result<Vec<u8>, FetchError> {
    fetch::get_with(url, opts)
}

/// Whether `host` is `domain` or under it (`www.example.com`).
fn same_site(host: &str, domain: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let domain = domain.split(':').next().unwrap_or(domain);
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|rest| rest.ends_with('.'))
}

/// The domain's icon: its home page's icon links, then `/favicon.ico`.
/// `origin` is the domain (with a port, in tests). `None`: none found.
pub fn fetch(origin: &str, get: Get) -> Option<Icon> {
    let domain = origin
        .split(':')
        .next()
        .unwrap_or(origin)
        .to_ascii_lowercase();
    let ok = |u: &Url| u.host_str().is_some_and(|h| same_site(h, &domain));
    let page_url = Url::parse(&format!("https://{origin}/")).ok()?;
    let opts = |accept: &'static str, max_body: usize| Options {
        accept,
        max_body,
        total: TIMEOUT,
        redirect_ok: &ok,
        user_agent: fetch::USER_AGENT,
        progress: None,
    };
    let mut tried: Vec<Url> = Vec::new();
    if let Ok(page) = get(page_url.as_str(), &opts("text/html", MAX_PAGE)) {
        let html = String::from_utf8_lossy(&page);
        for url in candidates(&page_url, &parse_links(&html), &domain)
            .into_iter()
            .take(MAX_LINKS)
        {
            if let Some(icon) = get(url.as_str(), &opts("image/*", MAX_ICON))
                .ok()
                .and_then(|b| decode(&b).ok())
            {
                return Some(icon);
            }
            tried.push(url);
        }
    }
    let ico = page_url.join("/favicon.ico").ok()?;
    if tried.contains(&ico) {
        return None;
    }
    get(ico.as_str(), &opts("image/*", MAX_ICON))
        .ok()
        .and_then(|b| decode(&b).ok())
}

/// An icon link of a page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub href: String,
    /// `rel="apple-touch-icon"` (180 px unless it says).
    pub apple: bool,
    /// The sizes it declares (`any` is left out).
    pub sizes: Vec<(u32, u32)>,
    /// Its `type`, lower case.
    pub mime: String,
}

/// The icon links of a page's HTML (`<link rel="icon">`, `shortcut icon`,
/// `apple-touch-icon`, `apple-touch-icon-precomposed`).
pub fn parse_links(html: &str) -> Vec<Link> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = lower[at..].find("<link") {
        let start = at + i + 5;
        let Some(len) = lower[start..].find('>') else {
            break;
        };
        let tag = &html[start..start + len];
        at = start + len;
        let attrs = attributes(tag);
        let get = |name: &str| {
            attrs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
        };
        let rel = get("rel").unwrap_or("").to_ascii_lowercase();
        let rels: Vec<&str> = rel.split_ascii_whitespace().collect();
        let apple = rels
            .iter()
            .any(|r| *r == "apple-touch-icon" || *r == "apple-touch-icon-precomposed");
        if !apple && !rels.contains(&"icon") {
            continue;
        }
        let Some(href) = get("href").map(str::trim).filter(|h| !h.is_empty()) else {
            continue;
        };
        let sizes = get("sizes")
            .unwrap_or("")
            .split_ascii_whitespace()
            .filter_map(|s| {
                let (w, h) = s
                    .to_ascii_lowercase()
                    .split_once('x')
                    .map(|(w, h)| (w.parse::<u32>().ok(), h.parse::<u32>().ok()))?;
                Some((w?, h?))
            })
            .collect();
        out.push(Link {
            href: href.to_string(),
            apple,
            sizes,
            mime: get("type").unwrap_or("").trim().to_ascii_lowercase(),
        });
    }
    out
}

/// A tag's attributes, names lower case, values unquoted (no entities
/// beyond `&amp;`).
fn attributes(tag: &str) -> Vec<(String, String)> {
    let b = tag.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let name_start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        if i == name_start {
            break;
        }
        let name = tag[name_start..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let s = i + 1;
                i = s;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                value = tag[s..i.min(tag.len())].to_string();
                i += 1;
            } else {
                let s = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                value = tag[s..i].to_string();
            }
        }
        out.push((name, value.replace("&amp;", "&")));
    }
    out
}

/// The side a link's icon is likely to have: the largest it declares, else
/// 180 for an apple-touch-icon and 32 for an icon.
fn likely_side(link: &Link) -> u32 {
    link.sizes
        .iter()
        .map(|&(w, h)| w.min(h))
        .max()
        .unwrap_or(if link.apple { 180 } else { 32 })
}

/// The side to choose for [`PIXELS`] among `sides`: the smallest at least
/// that large, else the largest. The index in `sides`.
pub fn best_side(sides: &[u32]) -> Option<usize> {
    let up = sides
        .iter()
        .enumerate()
        .filter(|(_, s)| **s >= PIXELS)
        .min_by_key(|(_, s)| **s);
    // On a tie, the first (the page's order).
    let down = sides
        .iter()
        .enumerate()
        .max_by_key(|(i, s)| (**s, std::cmp::Reverse(*i)));
    up.or(down).map(|(i, _)| i)
}

/// The icon URLs to try, best first: on the domain, not SVG, by how near
/// their size is to [`PIXELS`] (as [`best_side`]).
pub fn candidates(page: &Url, links: &[Link], domain: &str) -> Vec<Url> {
    let mut found: Vec<(u32, Url)> = links
        .iter()
        .filter(|l| !l.mime.contains("svg"))
        .filter_map(|l| {
            let u = page.join(&l.href).ok()?;
            let svg = u.path().to_ascii_lowercase().ends_with(".svg");
            let site = u.host_str().is_some_and(|h| same_site(h, domain));
            (u.scheme() == "https" && site && !svg).then_some((likely_side(l), u))
        })
        .collect();
    let mut out = Vec::new();
    while !found.is_empty() {
        let sides: Vec<u32> = found.iter().map(|(s, _)| *s).collect();
        let Some(i) = best_side(&sides) else { break };
        let (_, u) = found.remove(i);
        if !out.contains(&u) {
            out.push(u);
        }
    }
    out
}

// ---------------------------------------------------------------- decode

/// Why bytes are not an icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// SVG, HTML, or anything that is not a raster image.
    NotRaster,
    TooLarge,
    Undecodable,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            Refusal::NotRaster => "not a PNG, ICO, JPEG, GIF or BMP image",
            Refusal::TooLarge => "the image is too large",
            Refusal::Undecodable => "the image can't be read",
        })
    }
}

impl std::error::Error for Refusal {}

/// The largest file [`choose`] takes.
pub const MAX_CHOSEN: usize = 10 * 1024 * 1024;

/// Bytes made into an icon: sniffed by their magic (never a declared
/// type), the best ICO entry, scaled into [`PIXELS`] square.
pub fn decode(bytes: &[u8]) -> Result<Icon, Refusal> {
    if bytes.len() > MAX_CHOSEN {
        return Err(Refusal::TooLarge);
    }
    let rgba = if bytes.starts_with(&[0, 0, 1, 0]) {
        ico(bytes)?
    } else if bytes.starts_with(b"BM") {
        bmp_file(bytes)?
    } else {
        raster(bytes)?
    };
    Ok(finish(rgba))
}

/// Straight RGBA pixels.
struct Rgba {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

/// PNG, JPEG, GIF, WebP through the reader's decoders.
fn raster(bytes: &[u8]) -> Result<Rgba, Refusal> {
    let known = bytes.starts_with(&[0x89, b'P', b'N', b'G'])
        || bytes.starts_with(&[0xFF, 0xD8])
        || bytes.starts_with(b"GIF8")
        || (bytes.get(..4) == Some(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"));
    if !known {
        return Err(Refusal::NotRaster);
    }
    let mut budget = u64::from(MAX_SIDE) * u64::from(MAX_SIDE);
    let data = img::decode(bytes, 0, &mut budget).ok_or(Refusal::Undecodable)?;
    let (w, h) = (data.pix.width(), data.pix.height());
    if w > MAX_SIDE || h > MAX_SIDE {
        return Err(Refusal::TooLarge);
    }
    Ok(Rgba {
        w,
        h,
        px: demultiplied(data.pix.data()),
    })
}

fn demultiplied(premul: &[u8]) -> Vec<u8> {
    let mut px = premul.to_vec();
    for p in px.as_chunks_mut::<4>().0 {
        let a = u32::from(p[3]);
        if a != 0 && a != 255 {
            for c in p.iter_mut().take(3) {
                *c = (u32::from(*c) * 255 / a).min(255) as u8;
            }
        }
    }
    px
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// An ICO file: the entry whose size suits [`PIXELS`] best, PNG or DIB.
fn ico(b: &[u8]) -> Result<Rgba, Refusal> {
    let n = usize::from(u16_at(b, 4).ok_or(Refusal::Undecodable)?);
    let mut entries = Vec::new();
    for i in 0..n.min(64) {
        let e = 6 + i * 16;
        let side = |v: u8| if v == 0 { 256 } else { u32::from(v) };
        let (Some(&w), Some(&h)) = (b.get(e), b.get(e + 1)) else {
            break;
        };
        let (Some(size), Some(offset)) = (u32_at(b, e + 8), u32_at(b, e + 12)) else {
            break;
        };
        let bpp = u16_at(b, e + 6).unwrap_or(0);
        entries.push((side(w).min(side(h)), bpp, size as usize, offset as usize));
    }
    // The best size; among equal sizes the deepest colour.
    let sides: Vec<u32> = entries.iter().map(|e| e.0).collect();
    let best = best_side(&sides).ok_or(Refusal::Undecodable)?;
    let side = entries[best].0;
    let (_, _, size, offset) = entries
        .iter()
        .filter(|e| e.0 == side)
        .max_by_key(|e| e.1)
        .copied()
        .ok_or(Refusal::Undecodable)?;
    let data = b
        .get(offset..offset.checked_add(size).ok_or(Refusal::Undecodable)?)
        .ok_or(Refusal::Undecodable)?;
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        raster(data)
    } else {
        dib(data, true)
    }
}

/// A BMP file: its DIB after the 14-byte file header.
fn bmp_file(b: &[u8]) -> Result<Rgba, Refusal> {
    let pixels = u32_at(b, 10).ok_or(Refusal::Undecodable)? as usize;
    let dib_bytes = b.get(14..).ok_or(Refusal::Undecodable)?;
    let header = u32_at(dib_bytes, 0).ok_or(Refusal::Undecodable)? as usize;
    // Where the pixels start, after the header and the palette.
    dib_at(
        dib_bytes,
        false,
        pixels.checked_sub(14).filter(|&p| p >= header),
    )
}

fn dib(b: &[u8], icon: bool) -> Result<Rgba, Refusal> {
    dib_at(b, icon, None)
}

/// A device-independent bitmap (BITMAPINFOHEADER, BI_RGB or BI_BITFIELDS
/// at 32 bits; 1, 4, 8, 24 or 32 bits). In an ICO its height is doubled
/// and an AND mask follows (1: see-through).
fn dib_at(b: &[u8], icon: bool, pixels_at: Option<usize>) -> Result<Rgba, Refusal> {
    let bad = Refusal::Undecodable;
    let header = u32_at(b, 0).ok_or(bad)? as usize;
    if header < 40 {
        return Err(bad);
    }
    let w = i32::from_le_bytes(b.get(4..8).ok_or(bad)?.try_into().map_err(|_| bad)?);
    let raw_h = i32::from_le_bytes(b.get(8..12).ok_or(bad)?.try_into().map_err(|_| bad)?);
    let bpp = u16_at(b, 14).ok_or(bad)?;
    let compression = u32_at(b, 16).ok_or(bad)?;
    let colours = u32_at(b, 32).ok_or(bad)?;
    let top_down = raw_h < 0;
    let h_all = raw_h.unsigned_abs();
    let h = if icon { h_all / 2 } else { h_all };
    let w = u32::try_from(w).map_err(|_| bad)?;
    if w == 0 || h == 0 {
        return Err(bad);
    }
    if w > MAX_SIDE || h > MAX_SIDE {
        return Err(Refusal::TooLarge);
    }
    let rgb_ok = compression == 0 || (compression == 3 && bpp == 32);
    if !rgb_ok || ![1, 4, 8, 24, 32].contains(&bpp) {
        return Err(bad);
    }
    let palette_n = if bpp <= 8 {
        if colours == 0 {
            1usize << bpp
        } else {
            colours as usize
        }
        .min(256)
    } else {
        0
    };
    let palette_at = header;
    let palette = b.get(palette_at..palette_at + palette_n * 4).ok_or(bad)?;
    let start = pixels_at.unwrap_or(palette_at + palette_n * 4);
    let (wu, hu) = (w as usize, h as usize);
    let stride = (wu * usize::from(bpp)).div_ceil(32) * 4;
    let pixels = b.get(start..start + stride * hu).ok_or(bad)?;
    let mask_stride = wu.div_ceil(32) * 4;
    let mask = if icon {
        b.get(start + stride * hu..start + stride * hu + mask_stride * hu)
    } else {
        None
    };
    let mut px = vec![0u8; wu * hu * 4];
    let mut any_alpha = false;
    for y in 0..hu {
        let src_row = if top_down { y } else { hu - 1 - y };
        let row = &pixels[src_row * stride..(src_row + 1) * stride];
        for x in 0..wu {
            let (r, g, bl, a) = match bpp {
                32 => (row[x * 4 + 2], row[x * 4 + 1], row[x * 4], row[x * 4 + 3]),
                24 => (row[x * 3 + 2], row[x * 3 + 1], row[x * 3], 255),
                _ => {
                    let per = 8 / usize::from(bpp);
                    let byte = row[x / per];
                    let shift = 8 - usize::from(bpp) * (x % per + 1);
                    let i = usize::from((byte >> shift) & ((1u8 << bpp) - 1));
                    let c = palette.get(i * 4..i * 4 + 3).ok_or(bad)?;
                    (c[2], c[1], c[0], 255)
                }
            };
            if bpp == 32 && a != 0 {
                any_alpha = true;
            }
            let o = (y * wu + x) * 4;
            px[o..o + 4].copy_from_slice(&[r, g, bl, a]);
        }
    }
    // The AND mask gives the see-through pixels, unless 32-bit colour
    // carried its own alpha.
    if bpp < 32 || !any_alpha {
        for y in 0..hu {
            let src_row = if top_down { y } else { hu - 1 - y };
            for x in 0..wu {
                let clear = mask
                    .and_then(|m| m.get(src_row * mask_stride + x / 8))
                    .is_some_and(|byte| byte & (0x80 >> (x % 8)) != 0);
                px[(y * wu + x) * 4 + 3] = if clear { 0 } else { 255 };
            }
        }
    }
    Ok(Rgba { w, h, px })
}

/// Into [`PIXELS`] square (the aspect kept, centred on clear), the plate
/// judged, and PNG-encoded.
fn finish(src: Rgba) -> Icon {
    let side = PIXELS as usize;
    let scale =
        |v: u32, by: u32, of: u32| ((v as u64 * by as u64) / of.max(1) as u64).max(1) as u32;
    let (tw, th) = if src.w >= src.h {
        (PIXELS, scale(src.h, PIXELS, src.w))
    } else {
        (scale(src.w, PIXELS, src.h), PIXELS)
    };
    let scaled = resize(&src, tw, th);
    let mut out = vec![0u8; side * side * 4];
    let (ox, oy) = ((side - tw as usize) / 2, (side - th as usize) / 2);
    for y in 0..th as usize {
        let from = y * tw as usize * 4;
        let to = ((oy + y) * side + ox) * 4;
        out[to..to + tw as usize * 4].copy_from_slice(&scaled[from..from + tw as usize * 4]);
    }
    let (mut clear, mut solid, mut luma) = (0u64, 0u64, 0u64);
    for p in out.as_chunks::<4>().0 {
        if p[3] < 128 {
            clear += 1;
        } else {
            solid += 1;
            luma += (u64::from(p[0]) * 299 + u64::from(p[1]) * 587 + u64::from(p[2]) * 114) / 1000;
        }
    }
    let plate = img::needs_plate((side * side) as u64, clear, solid, luma);
    Icon {
        png: encode_png(&out, PIXELS, PIXELS),
        plate,
    }
}

/// Straight RGBA resampled to (tw, th): area average down, bilinear up
/// (the reader's resampler, on premultiplied pixels).
fn resize(src: &Rgba, tw: u32, th: u32) -> Vec<u8> {
    let mut pre = src.px.clone();
    for p in pre.as_chunks_mut::<4>().0 {
        let a = u32::from(p[3]);
        for c in p.iter_mut().take(3) {
            *c = ((u32::from(*c) * a + 127) / 255) as u8;
        }
    }
    let scaled = tiny_skia::IntSize::from_wh(src.w, src.h)
        .and_then(|size| tiny_skia::Pixmap::from_vec(pre, size))
        .and_then(|pix| img::resample(&pix, tw, th));
    match scaled {
        Some(p) => demultiplied(p.data()),
        None => vec![0; tw as usize * th as usize * 4],
    }
}

/// Straight RGBA w by h as a PNG (empty if it can't be written).
pub fn encode_png(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    if let Ok(mut writer) = enc.write_header() {
        writer.write_image_data(rgba).ok();
        writer.finish().ok();
    }
    out
}

// ---------------------------------------------------------------- base64

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(B64[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes().filter(|c| *c != b'=') {
        let v = B64.iter().position(|&b| b == c)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
#[path = "favicon_tests.rs"]
mod tests;
