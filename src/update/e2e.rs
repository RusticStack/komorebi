//! The update path end to end against a local HTTPS server: `check_from`
//! and `fetch_installer_from` run the production code (release JSON,
//! URL and redirect rules, caps, progress, hash and signature checks),
//! with only the base URL, the allowed hosts, the transport's trusted
//! certificate and the signing key injected (`Source`). The real app only
//! ever builds `Source::github()`.

use super::*;
use crate::mail::fake_tls::{CERT, KEY, acceptor};
use ed25519_compact::{KeyPair, Seed};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

const V_OLD: Version = Version(0, 4, 1);
const V_NEW: Version = Version(9, 1, 0);

type Routes = Arc<Mutex<HashMap<String, Vec<u8>>>>;

struct Server {
    port: u16,
    routes: Routes,
    /// The request heads received, in order.
    seen: Arc<Mutex<Vec<String>>>,
}

fn ok(body: &[u8]) -> Vec<u8> {
    let mut r = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    r.extend_from_slice(body);
    r
}

fn redirect(to: &str) -> Vec<u8> {
    format!("HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\n\r\n").into_bytes()
}

impl Server {
    fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes: Routes = Arc::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (r, s) = (routes.clone(), seen.clone());
        let acceptor = acceptor(CERT, KEY);
        std::thread::spawn(move || {
            for tcp in listener.incoming().flatten() {
                let (r, s, acceptor) = (r.clone(), s.clone(), acceptor.clone());
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
                    s.lock().unwrap().push(head);
                    let reply = r.lock().unwrap().get(&path).cloned();
                    let reply = reply.unwrap_or_else(|| {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec()
                    });
                    tls.write_all(&reply).ok();
                    tls.flush().ok();
                    tls.shutdown().ok();
                });
            }
        });
        Server { port, routes, seen }
    }

    fn base(&self) -> String {
        format!("https://127.0.0.1:{}", self.port)
    }

    fn releases(&self) -> String {
        format!("{}/releases/", self.base())
    }

    fn put(&self, path: &str, reply: Vec<u8>) {
        self.routes.lock().unwrap().insert(path.to_string(), reply);
    }

    fn paths(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|h| h.split(' ').nth(1).unwrap_or("").to_string())
            .collect()
    }

    /// Publishes a release `tag` whose files are the given bytes.
    fn publish(&self, tag: &str, version: Version, installer: &[u8], sha: &[u8], sig: &[u8]) {
        let name = installer_name(version);
        let base = format!("{}download/{tag}/", self.releases());
        let asset =
            |n: &str| format!(r#"{{"name":"{n}","size":1,"browser_download_url":"{base}{n}"}}"#);
        let json = format!(
            r#"{{"html_url":"{}tag/{tag}","tag_name":"{tag}","draft":false,"prerelease":false,"assets":[{},{},{}],"body":"Notes"}}"#,
            self.releases(),
            asset(&name),
            asset(&format!("{name}.sha256")),
            asset(&format!("{name}.sig")),
        );
        self.put("/latest", ok(json.as_bytes()));
        let path = |n: &str| format!("/releases/download/{tag}/{n}");
        self.put(&path(&name), ok(installer));
        self.put(&path(&format!("{name}.sha256")), ok(sha));
        self.put(&path(&format!("{name}.sig")), ok(sig));
    }
}

fn key() -> KeyPair {
    KeyPair::from_seed(Seed::new([21; 32]))
}

/// The `.sha256` and `.sig` files for `data` as release `v`, signed by `kp`.
fn sign(kp: &KeyPair, v: Version, data: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let sha = sha256_hex(data);
    let sig = kp.sk.sign(signed_message(v, &sha), None);
    (
        format!("{sha}  {}\n", installer_name(v)).into_bytes(),
        to_hex(sig.as_ref()).into_bytes(),
    )
}

/// Runs `f` with a `Source` for `server`: redirects may only reach
/// 127.0.0.1 (the stand-in for GitHub's hosts).
fn with_source<R>(
    server: &Server,
    kp: &KeyPair,
    running: Version,
    f: impl FnOnce(&Source) -> R,
) -> R {
    let latest = format!("{}/latest", server.base());
    let releases = server.releases();
    let host_ok = |u: &url::Url| u.host_str() == Some("127.0.0.1");
    let get = |u: &str, o: &Options| fetch::get_with_loopback(u, o, CERT);
    f(&Source {
        latest: &latest,
        releases: &releases,
        api_ok: &host_ok,
        asset_ok: &host_ok,
        get: &get,
        key: *kp.pk,
        running,
    })
}

fn temp() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "komorebi-update-e2e-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn files_under(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |d| d.flatten().count())
}

/// An installer a few blocks long, so progress arrives in several steps.
fn installer_bytes() -> Vec<u8> {
    (0..300_000u32).map(|i| (i % 251) as u8).collect()
}

#[test]
fn a_newer_release_is_offered_and_installed_with_progress() {
    let (server, kp) = (Server::start(), key());
    let exe = installer_bytes();
    let (sha, sig) = sign(&kp, V_NEW, &exe);
    server.publish("v9.1.0", V_NEW, &exe, &sha, &sig);
    let dir = temp();
    with_source(&server, &kp, V_OLD, |src| {
        let rel = check_from(src)
            .unwrap()
            .expect("a newer release is offered");
        assert_eq!(rel.version, V_NEW);
        assert_eq!(rel.installer.name, installer_name(V_NEW));

        let steps = std::cell::RefCell::new(Vec::new());
        let path = fetch_installer_from(src, &rel, &dir, &|done, total| {
            steps.borrow_mut().push((done, total));
        })
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), exe);
        assert!(path.starts_with(&dir));

        let steps = steps.into_inner();
        assert!(steps.len() >= 2, "{steps:?}");
        let total = Some(exe.len() as u64);
        assert!(steps.iter().all(|&(_, t)| t == total), "{steps:?}");
        assert!(steps.windows(2).all(|w| w[0].0 <= w[1].0), "{steps:?}");
        assert_eq!(steps.last().unwrap().0, exe.len() as u64);
    });
    // The check names the app and nothing else about the person.
    let heads = server.seen.lock().unwrap().clone();
    let first = heads.first().unwrap().to_ascii_lowercase();
    assert!(first.contains("\r\nuser-agent: komorebi/"), "{first}");
    assert!(!first.contains("\r\ncookie:") && !first.contains("\r\nauthorization:"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_running_version_is_not_offered_again() {
    let (server, kp) = (Server::start(), key());
    let exe = installer_bytes();
    let (sha, sig) = sign(&kp, V_OLD, &exe);
    server.publish("v0.4.1", V_OLD, &exe, &sha, &sig);
    with_source(&server, &kp, V_OLD, |src| {
        assert_eq!(check_from(src), Ok(None));
    });
    // An older release is not offered either.
    with_source(&server, &kp, Version(0, 5, 0), |src| {
        assert_eq!(check_from(src), Ok(None));
    });
    // Nothing was downloaded: only the release JSON was requested.
    assert_eq!(server.paths(), ["/latest", "/latest"]);
}

#[test]
fn a_tampered_installer_is_rejected_and_not_kept() {
    let (server, kp) = (Server::start(), key());
    let exe = installer_bytes();
    let (sha, sig) = sign(&kp, V_NEW, &exe);
    let mut evil = exe.clone();
    evil[1234] ^= 1;
    server.publish("v9.1.0", V_NEW, &evil, &sha, &sig);
    let dir = temp();
    with_source(&server, &kp, V_OLD, |src| {
        let rel = check_from(src).unwrap().unwrap();
        assert_eq!(
            fetch_installer_from(src, &rel, &dir, &|_, _| {}),
            Err(UpdateError::HashMismatch)
        );
    });
    assert_eq!(
        files_under(&dir),
        0,
        "nothing is written for a bad download"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_bad_signature_is_rejected() {
    let (server, kp) = (Server::start(), key());
    let exe = installer_bytes();
    let dir = temp();
    // Signed by another key, signed for another version (a replayed older
    // release), and a signature that is not even hex.
    let other = KeyPair::from_seed(Seed::new([22; 32]));
    let cases = [
        sign(&other, V_NEW, &exe),
        sign(&kp, Version(9, 0, 0), &exe),
        (sign(&kp, V_NEW, &exe).0, b"not a signature".to_vec()),
    ];
    for (sha, sig) in cases {
        server.publish("v9.1.0", V_NEW, &exe, &sha, &sig);
        with_source(&server, &kp, V_OLD, |src| {
            let rel = check_from(src).unwrap().unwrap();
            assert_eq!(
                fetch_installer_from(src, &rel, &dir, &|_, _| {}),
                Err(UpdateError::BadSignature)
            );
        });
    }
    assert_eq!(files_under(&dir), 0);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn redirects_stay_on_allowed_hosts() {
    let (server, kp) = (Server::start(), key());
    let exe = installer_bytes();
    let (sha, sig) = sign(&kp, V_NEW, &exe);
    server.publish("v9.1.0", V_NEW, &exe, &sha, &sig);
    let name = installer_name(V_NEW);
    let url = format!("/releases/download/v9.1.0/{name}");
    let dir = temp();

    // An allowed host (the stand-in for GitHub's CDN): followed.
    server.put(&url, redirect(&format!("{}/cdn/installer", server.base())));
    server.put("/cdn/installer", ok(&exe));
    with_source(&server, &kp, V_OLD, |src| {
        let rel = check_from(src).unwrap().unwrap();
        let path = fetch_installer_from(src, &rel, &dir, &|_, _| {}).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), exe);
    });

    // A host that is not on the list ("localhost" is the same server but
    // not an allowed name): refused before any request goes there.
    server.put(
        &url,
        redirect(&format!("https://localhost:{}/evil/installer", server.port)),
    );
    server.put("/evil/installer", ok(&exe));
    with_source(&server, &kp, V_OLD, |src| {
        let rel = check_from(src).unwrap().unwrap();
        assert_eq!(
            fetch_installer_from(src, &rel, &dir, &|_, _| {}),
            Err(UpdateError::Network)
        );
    });
    assert!(!server.paths().contains(&"/evil/installer".to_string()));

    // The same for the release JSON, and for a downgrade to plain http.
    server.put(
        "/latest",
        redirect(&format!("https://localhost:{}/evil/latest", server.port)),
    );
    server.put("/evil/latest", ok(b"{}"));
    with_source(&server, &kp, V_OLD, |src| {
        assert_eq!(check_from(src), Err(UpdateError::Network));
    });
    server.put("/latest", redirect("http://127.0.0.1:1/latest"));
    with_source(&server, &kp, V_OLD, |src| {
        assert_eq!(check_from(src), Err(UpdateError::Network));
    });
    assert!(!server.paths().contains(&"/evil/latest".to_string()));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn assets_outside_the_releases_prefix_are_never_fetched() {
    let (server, kp) = (Server::start(), key());
    let exe = installer_bytes();
    let (sha, sig) = sign(&kp, V_NEW, &exe);
    server.publish("v9.1.0", V_NEW, &exe, &sha, &sig);
    let dir = temp();
    with_source(&server, &kp, V_OLD, |src| {
        let mut rel = check_from(src).unwrap().unwrap();
        rel.installer.url = format!("{}/elsewhere/{}", server.base(), installer_name(V_NEW));
        assert_eq!(
            fetch_installer_from(src, &rel, &dir, &|_, _| {}),
            Err(UpdateError::BadRelease)
        );
    });
    assert!(!server.paths().iter().any(|p| p.starts_with("/elsewhere")));
    std::fs::remove_dir_all(&dir).ok();
}
