//! Converts a throwaway cache to the encrypted form or back
//! (`src/cachecrypt.rs`), for measurements until the setting lands. The
//! key goes to `<cache>.key` under DPAPI for the current Windows user.
//! Never point it at a real cache.
//!
//! ```text
//! cargo run --example cache_crypt -- <throwaway.sqlite3> on|off
//! ```

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let (Some(db), Some(mode)) = (args.next(), args.next()) else {
        anyhow::bail!("usage: cache_crypt <throwaway.sqlite3> on|off");
    };
    #[cfg(windows)]
    komorebi::cachecrypt::migrate(std::path::Path::new(&db), mode == "on")?;
    println!("{db}: {mode}");
    Ok(())
}
