//! Cache read timing on throwaway copies of a seeded cache, to judge SQLite
//! settings. `#[ignore]`d: `KOMOREBI_BENCH_DB=<seeded .sqlite3> cargo test
//! --release --test db_bench -- --ignored --nocapture`. The seeded file is
//! copied to the temp dir first and never opened itself.
//!
//! Findings (13700KF, release): first list on the 2k cache 2.3 ms whatever the
//! setting; on the 55k cache 9.9 ms default, 7.4 ms with `mmap_size` 256M,
//! cache_size and temp_store no change. The list, unread and id queries
//! already run on covering indexes (`list_plans`), and 20,000 headers seed in
//! about 131 ms (`insert_throughput`). No setting was changed: the mmap gain is
//! off the first-frame path and carries the Windows risk docs/PERF.md names.
//!
//! Also measured and left alone: `Rows::row` costs 4.3 us per row on a cold
//! read (30 visible rows, 0.13 ms a frame, `visible_rows`), too little for a
//! row cache and its invalidation to pay; `straight_bgra` converts a 900 x 256
//! piece of worst-case random alpha in 0.38 ms (`straight_bgra_piece`), and a
//! multiply-by-reciprocal version (exact for every pair) saved 8 percent, so
//! it was dropped.

use rusqlite::{Connection, OpenFlags};
use std::path::PathBuf;
use std::time::Instant;

const SETTINGS: [(&str, &str); 5] = [
    ("default", ""),
    ("cache_size=-16000", "PRAGMA cache_size = -16000;"),
    ("mmap_size=256M", "PRAGMA mmap_size = 268435456;"),
    ("temp_store=MEMORY", "PRAGMA temp_store = MEMORY;"),
    (
        "all three",
        "PRAGMA cache_size = -16000; PRAGMA mmap_size = 268435456; PRAGMA temp_store = MEMORY;",
    ),
];

fn copy_of(src: &str, tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("komorebi-db-bench-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dst = dir.join("cache.sqlite3");
    std::fs::copy(src, &dst).unwrap();
    dst
}

/// What the app reads to show the first list: folders, unread counts, the
/// ids of the largest folder, and the first rows' headers.
fn first_list(conn: &Connection) -> usize {
    let folders: Vec<(i64, i64)> = conn
        .prepare("SELECT f.id, (SELECT COUNT(*) FROM messages m WHERE m.folder_id = f.id) FROM folders f")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let inbox = folders.iter().max_by_key(|f| f.1).unwrap().0;
    let unread: usize = conn
        .prepare_cached(
            "SELECT folder_id, COUNT(*) FROM messages WHERE seen = 0 GROUP BY folder_id",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, i64>(1))
        .unwrap()
        .map(|r| r.unwrap() as usize)
        .sum();
    let ids: Vec<i64> = conn
        .prepare_cached(&format!(
            "SELECT id FROM messages WHERE folder_id IN ({inbox}) ORDER BY date DESC, id DESC"
        ))
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut bytes = unread + ids.len();
    for id in ids.iter().take(40) {
        bytes += conn
            .prepare_cached(
                "SELECT subject, from_name, from_addr, date, seen, flagged, answered,
                        has_attach, preview, to_addrs,
                        EXISTS(SELECT 1 FROM drafts WHERE reply_to_msg = messages.id AND state = 0),
                        folder_id, account_id, priority
                 FROM messages WHERE id = ?1",
            )
            .unwrap()
            .query_row([id], |r| Ok(r.get::<_, String>(0)?.len()))
            .unwrap();
    }
    bytes
}

#[test]
#[ignore = "timing; needs KOMOREBI_BENCH_DB"]
fn first_list_settings() {
    let Ok(seed) = std::env::var("KOMOREBI_BENCH_DB") else {
        println!("set KOMOREBI_BENCH_DB to a seeded cache");
        return;
    };
    let path = copy_of(&seed, "list");
    let mut times: Vec<Vec<f64>> = vec![vec![]; SETTINGS.len()];
    let mut check = None;
    for _round in 0..25 {
        for (i, (_, pragma)) in SETTINGS.iter().enumerate() {
            let t = Instant::now();
            let conn = Connection::open_with_flags(&path, OpenFlags::default()).unwrap();
            conn.execute_batch(
                "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;
                 PRAGMA journal_size_limit = 16777216;",
            )
            .unwrap();
            conn.execute_batch(pragma).unwrap();
            let got = first_list(&conn);
            times[i].push(t.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(*check.get_or_insert(got), got);
        }
    }
    for ((name, _), mut v) in SETTINGS.iter().zip(times) {
        v.sort_by(f64::total_cmp);
        println!(
            "{name:>20}: median {:.2} ms, min {:.2}, max {:.2}",
            v[v.len() / 2],
            v[0],
            v[v.len() - 1]
        );
    }
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
#[ignore = "prints query plans; needs KOMOREBI_BENCH_DB"]
fn list_plans() {
    let Ok(seed) = std::env::var("KOMOREBI_BENCH_DB") else {
        return;
    };
    let path = copy_of(&seed, "plan");
    let conn = Connection::open_with_flags(&path, OpenFlags::default()).unwrap();
    for sql in [
        "SELECT id FROM messages WHERE folder_id IN (1) ORDER BY date DESC, id DESC",
        "SELECT id FROM messages INDEXED BY messages_folder_seen WHERE folder_id IN (1) AND seen = 0 ORDER BY date DESC, id DESC",
        "SELECT folder_id, COUNT(*) FROM messages WHERE seen = 0 GROUP BY folder_id",
        "SELECT id, account_id, folder_id, uid, seen, flagged FROM messages WHERE id IN (1,2,3)",
        "SELECT id FROM messages WHERE folder_id IN (1) AND flagged = 1 ORDER BY date DESC, id DESC",
    ] {
        println!("{sql}");
        let mut st = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let rows = st.query_map([], |r| r.get::<_, String>(3)).unwrap();
        for r in rows {
            println!("    {}", r.unwrap());
        }
    }
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// Sync write throughput: `seed` stores generated headers in batches (one
/// transaction each, address index included) into a new throwaway cache,
/// then bodies for the newest 300.
#[test]
#[ignore = "timing"]
fn insert_throughput() {
    use komorebi::db::Db;
    let dir = std::env::temp_dir().join(format!("komorebi-db-bench-seed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut v = vec![];
    for round in 0..5 {
        let path = dir.join(format!("cache{round}.sqlite3"));
        let mut db = Db::open(&path).unwrap();
        let t = Instant::now();
        komorebi::seed::seed(&mut db, 20_000).unwrap();
        v.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    v.sort_by(f64::total_cmp);
    println!(
        "seed 20,000 headers: median {:.0} ms ({:.0} headers/s), min {:.0}, max {:.0}",
        v[2],
        20_000.0 / (v[2] / 1000.0),
        v[0],
        v[4]
    );
    std::fs::remove_dir_all(dir).ok();
}

/// What one list frame costs in `Rows::row`: the visible window (30 rows)
/// read as the list does, on a copy of a seeded cache.
#[test]
#[ignore = "timing; needs KOMOREBI_BENCH_DB"]
fn visible_rows() {
    use komorebi::db::Db;
    use komorebi::rows::{Kind, Rows};
    use std::rc::Rc;
    let Ok(seed) = std::env::var("KOMOREBI_BENCH_DB") else {
        return;
    };
    let path = copy_of(&seed, "rows");
    let ids: Vec<i64> = {
        let conn = Connection::open_with_flags(&path, OpenFlags::default()).unwrap();
        let inbox: i64 = conn
            .query_row(
                "SELECT folder_id FROM messages GROUP BY folder_id ORDER BY COUNT(*) DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        conn.prepare("SELECT id FROM messages WHERE folder_id = ?1 ORDER BY date DESC, id DESC")
            .unwrap()
            .query_map([inbox], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let rows = Rows::new(Rc::new(Db::open(&path).unwrap()));
    rows.set(ids, Kind::Received);
    let mut frame = vec![];
    for start in (0..300).step_by(7) {
        let t = Instant::now();
        let mut n = 0;
        for i in start..start + 30 {
            n += usize::from(rows.row(i).is_some());
        }
        assert_eq!(n, 30);
        frame.push(t.elapsed().as_secs_f64() * 1e6);
    }
    frame.sort_by(f64::total_cmp);
    println!(
        "30 visible rows: median {:.0} us per frame ({:.1} us per row), min {:.0}, max {:.0}",
        frame[frame.len() / 2],
        frame[frame.len() / 2] / 30.0,
        frame[0],
        frame[frame.len() - 1]
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// `compose::rich::straight_bgra` on a piece of text-like pixels (mostly
/// transparent, some edges, some solid), 900 x 256 px.
#[test]
#[ignore = "timing"]
fn straight_bgra_piece() {
    let (w, h) = (900u32, 256u32);
    let mut seed = 0x9E37_79B9u32;
    let mut times = vec![];
    for _ in 0..21 {
        let mut pm = tiny_skia::Pixmap::new(w, h).unwrap();
        for px in pm.data_mut().chunks_exact_mut(4) {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let a = match seed >> 24 {
                0..=199 => 0u8,
                200..=219 => 255,
                v => v as u8,
            };
            px.copy_from_slice(&[a / 2, a / 3, a / 4, a]);
        }
        let t = Instant::now();
        let out = komorebi::compose::rich::straight_bgra(pm);
        times.push(t.elapsed().as_secs_f64() * 1e6);
        std::hint::black_box(out);
    }
    times.sort_by(f64::total_cmp);
    println!(
        "straight_bgra 900x256: median {:.0} us, min {:.0}",
        times[10], times[0]
    );
}
