//! Release signing for the in-app update (src/update.rs).
//!
//!   cargo run --example sign_update -- keygen
//!   cargo run --example sign_update -- pubkey
//!   cargo run --example sign_update -- sign <installer.exe> <version>
//!   cargo run --example sign_update -- verify <installer.exe> <version>
//!
//! `keygen` writes a new private key to
//! `%USERPROFILE%\.komorebi-release\update-signing.key` (never inside the
//! repository; it refuses to overwrite an existing key) and prints only the
//! public key, which goes into `PUBLIC_KEY` in src/update.rs. `sign` writes
//! `<installer>.sha256` and `<installer>.sig` beside the installer; upload
//! both with it to the GitHub release. The private key is never printed.

use ed25519_compact::{KeyPair, Seed};
use komorebi::update;
use std::path::PathBuf;

fn key_path() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").expect("USERPROFILE is not set");
    PathBuf::from(home)
        .join(".komorebi-release")
        .join("update-signing.key")
}

fn load() -> KeyPair {
    let text = std::fs::read_to_string(key_path()).expect("cannot read the signing key");
    let seed = update::from_hex(text.trim()).expect("the signing key is not hex");
    let seed: [u8; 32] = seed.try_into().expect("the signing key is not 32 bytes");
    KeyPair::from_seed(Seed::new(seed))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("keygen") => {
            let path = key_path();
            if path.exists() {
                eprintln!("{} exists; not overwritten", path.display());
                std::process::exit(1);
            }
            std::fs::create_dir_all(path.parent().unwrap()).expect("cannot create the key folder");
            let kp = KeyPair::generate();
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .expect("cannot create the key file");
            std::io::Write::write_all(&mut f, update::to_hex(kp.sk.seed().as_ref()).as_bytes())
                .expect("cannot write the key file");
            println!("private key written to {}", path.display());
            println!("public key: {}", update::to_hex(kp.pk.as_ref()));
        }
        Some("pubkey") => println!("{}", update::to_hex(load().pk.as_ref())),
        Some("sign") if args.len() == 3 => {
            let file = PathBuf::from(&args[1]);
            let version = update::parse_version(&args[2]).expect("not a version like 0.4.0");
            let bytes = std::fs::read(&file).expect("cannot read the installer");
            let name = file.file_name().unwrap().to_string_lossy().into_owned();
            let sha = update::sha256_hex(&bytes);
            let kp = load();
            let sig = kp.sk.sign(update::signed_message(version, &sha), None);
            let mut sha_path = file.clone().into_os_string();
            sha_path.push(".sha256");
            let mut sig_path = file.clone().into_os_string();
            sig_path.push(".sig");
            std::fs::write(&sha_path, format!("{sha}  {name}\n")).expect("cannot write .sha256");
            std::fs::write(&sig_path, format!("{}\n", update::to_hex(sig.as_ref())))
                .expect("cannot write .sig");
            println!("signed {name} as version {version}");
        }
        Some("verify") if args.len() == 3 => {
            let file = PathBuf::from(&args[1]);
            let version = update::parse_version(&args[2]).expect("not a version like 0.4.0");
            let side = |ext: &str| {
                let mut p = file.clone().into_os_string();
                p.push(ext);
                std::fs::read(p).expect("cannot read the .sha256 or .sig file")
            };
            let bytes = std::fs::read(&file).expect("cannot read the installer");
            match update::verify(
                &bytes,
                &side(".sha256"),
                &side(".sig"),
                version,
                &update::PUBLIC_KEY,
            ) {
                Ok(()) => println!("verified against the embedded public key"),
                Err(e) => {
                    println!("verification FAILED: {e:?}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!(
                "usage: sign_update keygen | pubkey | sign <installer.exe> <version> | verify <installer.exe> <version>"
            );
            std::process::exit(2);
        }
    }
}
