//! Times the cache's hot read queries (id list, Unread and Flagged filters,
//! unread counts, search) on a copy of a seeded throwaway cache, for build
//! profile A/B runs (docs/PERF.md 4.2). Never point it at a real cache.
//!
//! ```text
//! cargo run --release --example db_bench -- <seeded.sqlite3> [runs]
//! ```

use anyhow::{Context, Result, bail};
use komorebi::db::{Db, Filter};
use std::time::Instant;

fn median_min(mut v: Vec<f64>) -> (f64, f64) {
    v.sort_by(f64::total_cmp);
    (v[v.len() / 2], v[0])
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(src) = args.next() else {
        bail!("usage: db_bench <seeded.sqlite3> [runs]");
    };
    let runs: usize = args.next().map_or(Ok(15), |r| r.parse())?;
    let copy =
        std::env::temp_dir().join(format!("komorebi-db-bench-{}.sqlite3", std::process::id()));
    std::fs::copy(&src, &copy).with_context(|| format!("copying {src}"))?;
    let (db, _) = Db::open_ui(&copy)?;
    let folders: Vec<i64> = {
        let c = rusqlite::Connection::open(&copy)?;
        let mut s = c.prepare(
            "SELECT folder_id FROM messages GROUP BY folder_id ORDER BY COUNT(*) DESC LIMIT 1",
        )?;
        s.query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    let report = |name: &str, f: &dyn Fn() -> Result<usize>| -> Result<()> {
        let n = f()?; // warm the page cache
        let mut t = Vec::new();
        for _ in 0..runs {
            let s = Instant::now();
            let m = f()?;
            t.push(s.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(m, n);
        }
        let (med, min) = median_min(t);
        println!("{name}: n={n} med={med:.2} min={min:.2} ms");
        Ok(())
    };
    report("ids all", &|| Ok(db.message_ids(&folders, "")?.len()))?;
    report("ids unread", &|| {
        Ok(db.message_ids_filtered(&folders, "", Filter::Unread)?.len())
    })?;
    report("ids flagged", &|| {
        Ok(db
            .message_ids_filtered(&folders, "", Filter::Flagged)?
            .len())
    })?;
    report("unread counts", &|| Ok(db.unread_counts()?.len()))?;
    report("search joao", &|| {
        Ok(db.message_ids(&folders, "joao")?.len())
    })?;
    report("search miss", &|| {
        Ok(db.message_ids(&folders, "zzqqxx")?.len())
    })?;
    report("search João", &|| {
        Ok(db.message_ids(&folders, "João")?.len())
    })?;
    drop(db);
    for ext in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{ext}", copy.display()));
    }
    let _ = std::fs::remove_dir_all(copy.with_extension("drafts"));
    Ok(())
}
