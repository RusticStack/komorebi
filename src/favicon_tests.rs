//! Account icons: the page's links, the size chosen, ICO and BMP, what is
//! refused (SVG, too large, a redirect off HTTPS or off the domain), the
//! order shown, and the switch that, off, makes no request. A local HTTPS
//! server stands in for the domain: no test touches the network.

use super::*;
use crate::db::NewAccount;
use crate::providers::{Security, Server};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

const CERT: &[u8] = include_bytes!("../tests/fixtures/tls/test-cert.pem");
const KEY: &[u8] = include_bytes!("../tests/fixtures/tls/test-key.pem");

/// A square PNG of `side`, every pixel `rgba`.
fn png(side: u32, rgba: [u8; 4]) -> Vec<u8> {
    let px: Vec<u8> = (0..side * side).flat_map(|_| rgba).collect();
    encode_png(&px, side, side)
}

/// A 32-bit DIB of `side` for an ICO (height doubled, an AND mask).
fn dib32(side: u32, bgra: [u8; 4]) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend(40u32.to_le_bytes());
    d.extend((side as i32).to_le_bytes());
    d.extend((side as i32 * 2).to_le_bytes());
    d.extend(1u16.to_le_bytes());
    d.extend(32u16.to_le_bytes());
    d.extend([0u8; 24]);
    for _ in 0..side * side {
        d.extend(bgra);
    }
    let mask_stride = (side as usize).div_ceil(32) * 4;
    d.extend(vec![0u8; mask_stride * side as usize]);
    d
}

/// An ICO of the entries (side, data).
fn ico_of(entries: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = vec![0, 0, 1, 0];
    out.extend((entries.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * entries.len();
    for (side, data) in entries {
        let s = if *side >= 256 { 0 } else { *side as u8 };
        out.extend([s, s, 0, 0]);
        out.extend(1u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend((data.len() as u32).to_le_bytes());
        out.extend((offset as u32).to_le_bytes());
        offset += data.len();
    }
    for (_, data) in entries {
        out.extend(data);
    }
    out
}

fn decoded(icon: &Icon) -> (u32, u32, Vec<u8>) {
    let mut dec = png::Decoder::new(std::io::Cursor::new(&icon.png[..]));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut r = dec.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

#[test]
fn icon_links_are_read_from_the_page() {
    let html = r#"<html><head>
        <LINK REL="stylesheet" href="/a.css">
        <link rel="icon" type="image/png" sizes="16x16 32x32" href="/i32.png">
        <link rel='shortcut icon' href=/favicon.ico>
        <link href="/touch.png" rel="apple-touch-icon">
        <link rel="icon" type="image/svg+xml" href="/logo.svg" sizes="any">
        <link rel="mask-icon" href="/mask.svg">
        <link rel="icon" href="">
    </head></html>"#;
    let links = parse_links(html);
    assert_eq!(links.len(), 4);
    assert_eq!(links[0].href, "/i32.png");
    assert_eq!(links[0].sizes, [(16, 16), (32, 32)]);
    assert_eq!(links[1].href, "/favicon.ico");
    assert!(links[2].apple && links[2].href == "/touch.png");
    assert_eq!(links[3].mime, "image/svg+xml");

    // Best first for 40 px: the 180 px touch icon, then the 32 px ones; the
    // SVG and a link to another site are never tried.
    let page = Url::parse("https://example.org/").unwrap();
    let mut with_other = links.clone();
    with_other.push(Link {
        href: "https://icons.example.net/a.png".into(),
        apple: true,
        sizes: vec![(64, 64)],
        mime: String::new(),
    });
    with_other.push(Link {
        href: "https://www.example.org/w.png".into(),
        apple: false,
        sizes: vec![(48, 48)],
        mime: String::new(),
    });
    let urls: Vec<String> = candidates(&page, &with_other, "example.org")
        .into_iter()
        .map(|u| u.to_string())
        .collect();
    assert_eq!(
        urls,
        [
            "https://www.example.org/w.png",
            "https://example.org/touch.png",
            "https://example.org/i32.png",
            "https://example.org/favicon.ico",
        ]
    );
}

#[test]
fn the_size_for_a_20_px_slot_at_200_percent() {
    assert_eq!(PIXELS, 40);
    // The smallest at least 40, else the largest.
    assert_eq!(best_side(&[16, 32, 48, 256]), Some(2));
    assert_eq!(best_side(&[256, 40, 64]), Some(1));
    assert_eq!(best_side(&[16, 32, 24]), Some(1));
    assert_eq!(best_side(&[]), None);
}

#[test]
fn ico_bmp_and_png_become_a_40_px_icon() {
    // ICO: the 48 px PNG entry over the 16 and 256 px ones.
    let ico = ico_of(&[
        (16, png(16, [255, 0, 0, 255])),
        (48, png(48, [0, 255, 0, 255])),
        (256, png(256, [0, 0, 255, 255])),
    ]);
    let icon = decode(&ico).unwrap();
    let (w, h, px) = decoded(&icon);
    assert_eq!((w, h), (40, 40));
    assert_eq!(&px[..4], &[0, 255, 0, 255]);
    // ICO with a 32-bit DIB entry.
    let icon = decode(&ico_of(&[(32, dib32(32, [0, 0, 200, 255]))])).unwrap();
    assert_eq!(&decoded(&icon).2[..4], &[200, 0, 0, 255]);
    // A BMP file, 24-bit, bottom-up, 2x1.
    let mut bmp = b"BM".to_vec();
    let pixels_at = 14 + 40;
    bmp.extend(((pixels_at + 8) as u32).to_le_bytes());
    bmp.extend([0u8; 4]);
    bmp.extend((pixels_at as u32).to_le_bytes());
    bmp.extend(40u32.to_le_bytes());
    bmp.extend(2i32.to_le_bytes());
    bmp.extend(1i32.to_le_bytes());
    bmp.extend(1u16.to_le_bytes());
    bmp.extend(24u16.to_le_bytes());
    bmp.extend([0u8; 24]);
    bmp.extend([255, 0, 0, 255, 0, 0, 0, 0]);
    let icon = decode(&bmp).unwrap();
    let (w, h, px) = decoded(&icon);
    assert_eq!((w, h), (40, 40));
    // Wider than tall: centred on clear, top rows see-through.
    assert_eq!(px[3], 0);
    assert_eq!(
        &px[(20 * 40 + 5) * 4..(20 * 40 + 5) * 4 + 4],
        &[0, 0, 255, 255]
    );
    // A black mark on clear wants the plate on the dark theme; white not.
    let mut mark = vec![0u8; 40 * 40 * 4];
    for p in mark.chunks_mut(4).step_by(3) {
        p.copy_from_slice(&[0, 0, 0, 255]);
    }
    assert!(decode(&encode_png(&mark, 40, 40)).unwrap().plate);
    assert!(!decode(&png(40, [255, 255, 255, 255])).unwrap().plate);
}

#[test]
fn svg_html_and_oversize_are_refused() {
    assert_eq!(
        decode(br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#),
        Err(Refusal::NotRaster)
    );
    assert_eq!(decode(b"<!doctype html><html>"), Err(Refusal::NotRaster));
    assert_eq!(
        decode(&png(2048, [1, 2, 3, 255])),
        Err(Refusal::Undecodable)
    );
    assert_eq!(decode(&vec![0u8; MAX_CHOSEN + 1]), Err(Refusal::TooLarge));
}

#[test]
fn kept_icons_survive_base64() {
    for data in [&b""[..], b"a", b"ab", b"abc", b"\x00\xff\x10\x80"] {
        assert_eq!(base64_decode(&base64_encode(data)).unwrap(), data);
    }
    assert_eq!(base64_encode(b"Man"), "TWFu");
    assert_eq!(base64_encode(b"Ma"), "TWE=");
}

// ---------------------------------------------------------------- server

/// The fake domain's answer to a path.
fn respond(path: &str, port: u16, good: bool) -> Vec<u8> {
    let ok = |kind: &str, body: Vec<u8>| {
        let mut r = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nSet-Cookie: t=1\r\n\r\n",
            body.len()
        )
        .into_bytes();
        r.extend(body);
        r
    };
    let redirect = |to: String| {
        format!("HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\n\r\n").into_bytes()
    };
    match path {
        // The good site: an SVG first, a big PNG, the touch icon.
        "/" if good => ok(
            "text/html",
            br#"<link rel="icon" href="/logo.svg" type="image/svg+xml">
                <link rel="icon" href="/big.png" sizes="64x64">
                <link rel="apple-touch-icon" href="/touch.png">"#
                .to_vec(),
        ),
        "/logo.svg" => ok(
            "image/svg+xml",
            br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#.to_vec(),
        ),
        // Over the size cap.
        "/big.png" => ok("image/png", vec![0x89; MAX_ICON + 1]),
        "/touch.png" => ok("image/png", png(180, [10, 20, 30, 255])),
        // A page off HTTPS, an icon that leaves for http.
        "/" => ok(
            "text/html",
            br#"<link rel="icon" href="/to-http.png">"#.to_vec(),
        ),
        "/to-http.png" => redirect(format!("http://127.0.0.1:{port}/touch.png")),
        "/favicon.ico" => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
        _ => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
    }
}

fn serve(good: bool) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let acceptor = crate::mail::fake_tls::acceptor(CERT, KEY);
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
                tls.write_all(&respond(&path, port, good)).ok();
                tls.flush().ok();
                tls.shutdown().ok();
            });
        }
    });
    (port, seen)
}

fn local(url: &str, opts: &Options) -> Result<Vec<u8>, FetchError> {
    crate::fetch::get_with_loopback(url, opts, CERT)
}

fn paths(seen: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    seen.lock()
        .unwrap()
        .iter()
        .map(|h| h.split(' ').nth(1).unwrap_or("").to_string())
        .collect()
}

#[test]
fn the_domain_icon_skips_svg_and_oversize_and_sends_nothing_personal() {
    let (port, seen) = serve(true);
    let icon = fetch(&format!("127.0.0.1:{port}"), &local).expect("the touch icon");
    let (w, h, px) = decoded(&icon);
    assert_eq!((w, h), (40, 40));
    assert_eq!(&px[..4], &[10, 20, 30, 255]);
    assert_eq!(
        paths(&seen),
        ["/", "/big.png", "/touch.png"],
        "the SVG is never asked; big.png is over the cap"
    );
    for head in seen.lock().unwrap().iter() {
        let lower = head.to_ascii_lowercase();
        assert!(!lower.contains("\r\ncookie:"), "{head}");
        assert!(!lower.contains("\r\nreferer:"), "{head}");
        assert!(head.contains("\r\nUser-Agent: Mozilla/5.0\r\n"), "{head}");
    }
}

#[test]
fn a_redirect_off_https_is_refused_and_nothing_is_found() {
    let (port, seen) = serve(false);
    assert_eq!(fetch(&format!("127.0.0.1:{port}"), &local), None);
    assert_eq!(paths(&seen), ["/", "/to-http.png", "/favicon.ico"]);
}

// ---------------------------------------------------------------- the order

fn account(db: &mut Db, email: &str, imap_host: &str) -> Account {
    let server = |host: &str| Server {
        host: host.into(),
        port: 993,
        security: Security::Tls,
    };
    let id = db
        .add_account(&NewAccount {
            imap_user: String::new(),
            smtp_user: String::new(),
            email: email.into(),
            name: String::new(),
            imap: server(imap_host),
            smtp: server(imap_host),
            save_sent: true,
            smtp_no_auth: false,
        })
        .unwrap();
    db.accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.id == id)
        .unwrap()
}

#[test]
fn the_order_is_chosen_domain_provider_nothing() {
    let mut db = Db::in_memory().unwrap();
    let gmail = providers::by_id("gmail").expect("a gmail preset");
    let own = account(&mut db, "me@example.org", gmail.imap.0);
    let plain = account(&mut db, "me@example.net", "imap.example.net");
    let on_preset = account(&mut db, &format!("me@{}", gmail.domains[0]), gmail.imap.0);
    // Nothing kept: the provider by the IMAP host, else nothing at all.
    assert_eq!(shown(&db, &own), Shown::Provider("gmail"));
    assert_eq!(shown(&db, &plain), Shown::None);
    // A provider's own domain is never asked.
    assert_eq!(domain_to_ask(&on_preset), None);
    assert_eq!(domain_to_ask(&own).as_deref(), Some("example.org"));
    assert!(!due(&db, &on_preset, 0));
    assert!(due(&db, &own, 0));
    // The domain's icon, then the user's over it.
    let fetched = decode(&png(40, [1, 2, 3, 255])).unwrap();
    keep(&db, own.id, Some(fetched.clone()), 100).unwrap();
    assert_eq!(shown(&db, &own), Shown::Image(fetched.clone()));
    assert!(!due(&db, &own, 100 + 60), "not asked again per launch");
    assert!(due(&db, &own, 100 + REFRESH.as_secs() as i64));
    choose(&db, own.id, &png(64, [9, 9, 9, 255]), 200).unwrap();
    let Shown::Image(chosen) = shown(&db, &own) else {
        panic!("the chosen image");
    };
    assert_ne!(chosen, fetched);
    assert!(
        !due(&db, &own, i64::MAX),
        "a chosen image is never replaced"
    );
    keep(&db, own.id, Some(fetched.clone()), 300).unwrap();
    assert_eq!(shown(&db, &own), Shown::Image(chosen));
    // Removed: the provider again until the domain is asked.
    remove(&db, own.id).unwrap();
    assert_eq!(shown(&db, &own), Shown::Provider("gmail"));
    // A domain without one: nothing kept to show, not asked again soon.
    keep(&db, plain.id, None, 400).unwrap();
    assert_eq!(shown(&db, &plain), Shown::None);
    assert!(!due(&db, &plain, 500));
    // The account goes: so does its icon.
    keep(&db, plain.id, Some(fetched), 600).unwrap();
    db.remove_account(plain.id).unwrap();
    assert_eq!(load(&db, plain.id), None);
}

#[test]
fn the_switch_off_makes_no_request_and_drops_fetched_icons() {
    let mut db = Db::in_memory().unwrap();
    let a = account(&mut db, "me@example.org", "imap.example.org");
    let b = account(&mut db, "you@example.com", "imap.example.com");
    let fetched = decode(&png(40, [1, 2, 3, 255])).unwrap();
    keep(&db, a.id, Some(fetched), 1).unwrap();
    choose(&db, b.id, &png(40, [4, 5, 6, 255]), 1).unwrap();
    assert!(enabled(&db));
    set_enabled(&db, false).unwrap();
    assert!(!enabled(&db));
    assert_eq!(load(&db, a.id), None, "the fetched icon is dropped");
    assert!(
        matches!(shown(&db, &b), Shown::Image(_)),
        "the chosen one stays"
    );
    // Off: never due, and what a fetch already under way finds is not kept.
    assert!(!due(&db, &a, i64::MAX));
    keep(
        &db,
        a.id,
        Some(decode(&png(40, [7, 7, 7, 255])).unwrap()),
        2,
    )
    .unwrap();
    assert_eq!(load(&db, a.id), None);
    // The app asks only what is due: with the switch off no request is
    // made at all (the GET below would be counted).
    let asked = std::cell::Cell::new(0);
    let counting = |_: &str, _: &Options| -> Result<Vec<u8>, FetchError> {
        asked.set(asked.get() + 1);
        Err(FetchError::Offline)
    };
    for acc in db.accounts().unwrap() {
        if due(&db, &acc, i64::MAX) {
            fetch(&domain_to_ask(&acc).unwrap(), &counting);
        }
    }
    assert_eq!(asked.get(), 0);
    set_enabled(&db, true).unwrap();
    assert!(due(&db, &a, 0));
}
