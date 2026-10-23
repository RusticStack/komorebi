//! Measure Komorebi's sync engine against a real account.
//!
//! ```text
//! cargo run --release --example probe -- <env-file> <throwaway-db> [--runs N] [--send [--idle]]
//! ```
//!
//! The env file holds `EMAIL` and `PASSWORD_EMAIL` (and optionally
//! `PROVIDER`). Safety: folders are opened read-only (EXAMINE) and every
//! mutating IMAP call is refused, so nothing in the mailbox changes. With
//! `--send`, exactly one `[komorebi-test]` message is sent to the account's
//! own address and a copy is appended to its Sent folder. With `--idle` as
//! well, a read-only connection waits in IDLE on the inbox while it is sent,
//! timing the wake and the sync pass after it. Output is numbers
//! and capability names only: no subjects, senders, or folder names.
//!
//! Each run starts from a fresh throwaway cache and signs in twice: once on a
//! separate connection that times the connect phases, then for the
//! measurements. With `--runs N` (at most 5, 2 s apart) the median of every
//! line follows the per-run output. Lines read `ms, B (IMAP payload), cmds`.

use anyhow::{Context, Result, anyhow, bail};
use komorebi::db::{Db, Folder, Fp, NewAccount};
use komorebi::mail::imap::{Imap, WaitOutcome};
use komorebi::mail::{self, mime, smtp};
use komorebi::providers::{PRESETS, Preset, Security, Server};
use komorebi::role::Role;
use native_tls::TlsConnector;
use std::collections::HashMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

const USAGE: &str = "usage: probe <env-file> <throwaway-db> [--runs N] [--send [--idle]]";
/// How long `--idle` waits for the test message to arrive.
const IDLE_WAIT: Duration = Duration::from_secs(60);
const MAX_RUNS: usize = 5;

fn main() -> Result<()> {
    let result = run();
    if let Err(e) = &result {
        // The crate's Display hides which response it rejected; Debug has it.
        if let Some(detail) = komorebi::mail::imap::detail(e) {
            eprintln!("imap error detail: {detail}");
        }
    }
    result
}

fn run() -> Result<()> {
    let (mut positional, mut send, mut idle, mut runs) = (Vec::new(), false, false, 1);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--send" => send = true,
            "--idle" => idle = true,
            "--runs" => {
                runs = args
                    .next()
                    .and_then(|n| n.parse().ok())
                    .context("--runs needs a number")?
            }
            _ => positional.push(a),
        }
    }
    let [env_path, db_path] = positional.as_slice() else {
        bail!(USAGE);
    };
    if !(1..=MAX_RUNS).contains(&runs) {
        bail!("--runs must be 1 to {MAX_RUNS}");
    }
    if idle && !send {
        bail!("--idle needs --send: it times the wake for the test message");
    }
    refuse_real_cache(db_path)?;
    let env = read_env(env_path)?;
    let probe = Probe {
        email: env.get("EMAIL").context("EMAIL missing")?.clone(),
        password: env
            .get("PASSWORD_EMAIL")
            .context("PASSWORD_EMAIL missing")?
            .clone(),
        preset: &PRESETS[1], // Postale.io
        db_path: db_path.clone(),
    };

    let mut report = Report::default();
    let mut folders = Vec::new();
    for run in 0..runs {
        if run > 0 {
            std::thread::sleep(Duration::from_secs(2));
        }
        if runs > 1 {
            println!("run {}/{runs}:", run + 1);
        }
        folders = probe.run(run == 0, &mut report)?;
    }
    if runs > 1 {
        println!("median of {runs} runs:");
        report.print_medians();
    }

    // ---- one test message to ourselves
    if send {
        let (email, password) = (&probe.email, &probe.password);
        // The IDLE connection only notifies, as in the app; the pass after the
        // wake runs on a separate, already signed-in read-only connection.
        let watcher = if idle {
            let mut syncer = Imap::connect(
                &probe.preset.imap_server(),
                None,
                &probe.email,
                &probe.password,
            )?;
            syncer.set_read_only(true);
            Some((start_idle(&probe, &folders)?, syncer))
        } else {
            None
        };
        let t = Instant::now();
        let msg = smtp::build(smtp::Outgoing {
            from_email: email.clone(),
            to: email.clone(),
            subject: format!("[komorebi-test] probe {}", chrono_free_stamp()),
            body: "Sent by Komorebi's probe to measure sending. Safe to delete.".into(),
            ..Default::default()
        })?;
        smtp::send(&probe.preset.smtp_server(), email, password, &msg)?;
        let send_ms = ms(t);
        if let Some((watcher, syncer)) = watcher {
            idle_wake(&probe, watcher, syncer, t)?;
        }
        let mut writer = Imap::connect(&probe.preset.imap_server(), None, email, password)?;
        let sent = folders.iter().find(|f| f.role == Role::Sent);
        let t = Instant::now();
        if let Some(sent) = sent {
            writer.append(&sent.name, &msg.formatted())?;
        }
        println!(
            "send to self: smtp {send_ms:.0} ms, append to Sent {:.0} ms (sent folder found: {})",
            ms(t),
            sent.is_some()
        );
        writer.logout();
    }
    Ok(())
}

struct Probe {
    email: String,
    password: String,
    preset: &'static Preset,
    db_path: String,
}

impl Probe {
    /// One full measurement on a fresh cache. Diagnostics that are not
    /// timings (capabilities, size histogram, parser checks) print only on
    /// the first run.
    fn run(&self, first: bool, report: &mut Report) -> Result<Vec<Folder>> {
        let server = self.preset.imap_server();
        let _ = std::fs::remove_file(&self.db_path);
        let mut db = Db::open(self.db_path.as_ref())?;
        let account_id = db.add_account(&NewAccount {
            imap_user: String::new(),
            smtp_user: String::new(),
            email: self.email.clone(),
            name: String::new(),
            imap: server.clone(),
            smtp: self.preset.smtp_server(),
            save_sent: true,
            smtp_no_auth: false,
        })?;

        // ---- connection
        connect_phases(&server, &self.email, &self.password, report)?;
        let t = Instant::now();
        let mut imap = Imap::connect(&server, None, &self.email, &self.password)?;
        imap.set_read_only(true);
        report.add(
            "connect+tls+login",
            Sample {
                ms: ms(t),
                bytes: Some(imap.bytes()),
                cmds: Some(imap.commands()),
            },
            "",
        );
        let caps: Vec<String> = imap.capabilities().to_vec();
        if first {
            print_capabilities(&caps);
        }

        // ---- round-trip floor: 20 NOOPs (nearest-rank median and p90)
        let mut rtts = Vec::new();
        for _ in 0..20 {
            let t = Instant::now();
            imap.noop()?;
            rtts.push(ms(t));
        }
        for (label, p) in [("rtt floor median", 0.5), ("rtt floor p90", 0.9)] {
            let s = Sample {
                ms: percentile(&rtts, p),
                bytes: None,
                cmds: Some(1),
            };
            report.add(label, s, "  (20 x NOOP)");
        }

        // ---- folder list
        let m = Mark::new(&imap);
        let (folders, statuses) = mail::refresh_folders(&mut imap, &db, account_id)?;
        report.add(
            "list folders",
            m.sample(&imap),
            &format!(
                "  {} folders, {} with STATUS",
                folders.len(),
                statuses.len()
            ),
        );

        // ---- first sync, per folder (always the full path)
        let mut total = Sample::default();
        for (i, f) in folders.iter().enumerate() {
            let label = format!("pass 1 {}", label(f.role, i));
            let m = Mark::new(&imap);
            if f.role == Role::Inbox {
                first_rows(&mut imap, &mut db, f, report)?;
            } else {
                mail::sync_folder_once(&mut imap, &mut db, f)?;
            }
            let s = m.sample(&imap);
            total.add(s);
            let n = db.known_uids(f.id)?.len();
            report.add(&label, s, &format!("  {n} msgs"));
        }
        report.add("pass 1 total", total, "");

        // ---- whole passes as the sync worker runs them: a forced full one
        // that records fingerprints, then a steady-state one that may skip
        for (pass, full) in [(2, true), (3, false)] {
            let m = Mark::new(&imap);
            let stats = mail::sync_pass_once(&mut imap, &mut db, account_id, full)?;
            report.add(
                &format!("pass {pass} total (full: {full})"),
                m.sample(&imap),
                &format!("  {} folders, {} skipped", stats.folders, stats.skipped),
            );
        }

        let inbox = folders
            .iter()
            .find(|f| f.role == Role::Inbox)
            .context("no inbox")?;
        if first {
            size_histogram(&self.db_path, inbox.id)?;
        }

        // ---- header fetch cost by batch size (same 500 newest inbox UIDs)
        let mut uids: Vec<u32> = db.known_uids(inbox.id)?.into_iter().collect();
        uids.sort_unstable_by(|a, b| b.cmp(a));
        uids.truncate(500);
        imap.select(&inbox.name)?;
        for batch in [10, 25, 50, 100, 250, 500] {
            let m = Mark::new(&imap);
            for chunk in uids.chunks(batch) {
                imap.fetch_headers(chunk)?;
            }
            report.add(
                &format!("headers, batches of {batch}"),
                m.sample(&imap),
                &format!("  {} msgs", uids.len()),
            );
        }

        // ---- inbox prefetch: one budgeted pass at a time until the window is full
        let policy = mail::PREFETCH;
        for pass in 1.. {
            let m = Mark::new(&imap);
            let stored = mail::prefetch_bodies(&mut imap, &mut db, inbox, &policy)?;
            report.add(
                &format!("prefetch pass {pass}"),
                m.sample(&imap),
                &format!("  {stored} bodies"),
            );
            if stored == 0 {
                break;
            }
        }
        let left = db.missing_bodies(inbox.id, policy.window, policy.max_msg)?;
        println!("missing in window after fill: {}", left.len());

        // ---- open an uncached message (fetch) and a cached one (parse only)
        let all_msgs = db.count_messages(inbox.id)? as usize;
        if let Some((id, uid, _)) = db
            .missing_bodies(inbox.id, all_msgs, i64::MAX)?
            .first()
            .copied()
        {
            let m = Mark::new(&imap);
            // UIDVALIDITY 0 skips the check; the probe only reads.
            let bodies = imap.fetch_bodies((&inbox.name, 0), &[uid])?;
            let size = bodies.first().map_or(0, |(_, r)| r.len());
            report.add(
                "open uncached: fetch",
                m.sample(&imap),
                &format!("  {size} B message"),
            );
            let t = Instant::now();
            if let Some((_, raw)) = bodies.first() {
                mail::store_body(&mut db, id, raw)?;
            }
            report.add("open uncached: parse+store", Sample::local(t), "");
        }
        let ids = db.message_ids(&[inbox.id], "")?;
        let mut parse = Vec::new();
        for id in ids.iter().take(50) {
            let t = Instant::now();
            if let Some(m) = db.message(*id)?
                && let Some(raw) = m.raw.as_deref()
            {
                let body = mime::parse_body(raw);
                std::hint::black_box(body);
                parse.push(ms(t));
            }
        }
        if !parse.is_empty() {
            let s = Sample {
                ms: percentile(&parse, 0.5),
                ..Sample::default()
            };
            let max = percentile(&parse, 1.0);
            let note = format!("  {} msgs, max {max:.2} ms", parse.len());
            report.add("open cached (db read + parse)", s, &note);
        }

        changed_inbox(&mut imap, &mut db, account_id, inbox, report)?;

        // ---- CONDSTORE: does imap-proto 0.10 parse MODSEQ responses?
        if first && caps.iter().any(|c| c.eq_ignore_ascii_case("CONDSTORE")) {
            imap.select(&inbox.name)?; // EXAMINE (read-only)
            match imap.raw("UID FETCH 1:* (UID) (CHANGEDSINCE 1)") {
                Ok(r) => println!(
                    "condstore CHANGEDSINCE raw: {} B, has MODSEQ: {}",
                    r.len(),
                    String::from_utf8_lossy(&r).contains("MODSEQ")
                ),
                Err(e) => println!("condstore raw command failed: {e}"),
            }
            match imap.fetch_count("1:5", "(UID FLAGS MODSEQ)") {
                Ok(n) => println!("imap-proto parses MODSEQ fetch: yes ({n} items)"),
                Err(e) => println!("imap-proto parses MODSEQ fetch: no ({e})"),
            }
        }

        // ---- local search over the cache
        let all: Vec<i64> = folders.iter().map(|f| f.id).collect();
        for term in ["a", "invoice", "zzzz-no-match"] {
            let t = Instant::now();
            let n = db.message_ids(&all, term)?.len();
            report.add(
                &format!("search {term:?}"),
                Sample::local(t),
                &format!("  {n} hits"),
            );
        }
        let size = std::fs::metadata(&self.db_path).map_or(0, |m| m.len());
        println!("cache file: {:.1} MB", size as f64 / 1e6);
        imap.logout();
        Ok(folders)
    }
}

/// A read-only connection in IDLE on the inbox, on a thread of its own.
/// It returns when the inbox changes or after [`IDLE_WAIT`], with the time
/// it woke and the connection.
type Watcher = std::thread::JoinHandle<(Result<WaitOutcome>, Instant, Imap)>;

fn start_idle(probe: &Probe, folders: &[Folder]) -> Result<Watcher> {
    let inbox = folders
        .iter()
        .find(|f| f.role == Role::Inbox)
        .context("no inbox")?;
    let mut imap = Imap::connect(
        &probe.preset.imap_server(),
        None,
        &probe.email,
        &probe.password,
    )?;
    if !imap.has_capability("IDLE") {
        bail!("the server has no IDLE");
    }
    imap.set_read_only(true);
    imap.select(&inbox.name)?; // EXAMINE
    Ok(std::thread::spawn(move || {
        let outcome = imap.idle(IDLE_WAIT);
        (outcome, Instant::now(), imap)
    }))
}

/// Wait for the IDLE wake for the message whose SMTP send began at `sent`,
/// then run a steady-state sync pass on the same connection (inbox first).
fn idle_wake(probe: &Probe, watcher: Watcher, mut imap: Imap, sent: Instant) -> Result<()> {
    let (outcome, woke, idler) = watcher
        .join()
        .map_err(|_| anyhow!("IDLE thread panicked"))?;
    let outcome = outcome.context("IDLE wait")?;
    idler.logout();
    if outcome == WaitOutcome::TimedOut {
        println!("idle: no wake within {} s", IDLE_WAIT.as_secs());
        imap.logout();
        return Ok(());
    }
    let wake_ms = woke.saturating_duration_since(sent).as_secs_f64() * 1000.0;
    let mut db = Db::open(probe.db_path.as_ref())?;
    let account_id = db.accounts()?.first().context("no account")?.id;
    let m = Mark::new(&imap);
    let stats = mail::sync_pass_once(&mut imap, &mut db, account_id, false)
        .context("sync pass after the wake")?;
    let s = m.sample(&imap);
    println!("idle: send→wake {wake_ms:.0} ms (from the start of the SMTP send)");
    println!(
        "{}  {} folders, {} skipped",
        line("idle: wake→rows (pass)", s),
        stats.folders,
        stats.skipped
    );
    imap.logout();
    Ok(())
}

/// The inbox's first sync as the sync worker runs it, timing when its
/// first header batch is stored (the first `FolderChanged`; a fresh cache
/// has nothing to remove first) as `inbox first rows`.
fn first_rows(imap: &mut Imap, db: &mut Db, f: &Folder, report: &mut Report) -> Result<()> {
    let t = Instant::now();
    let first = Arc::new(OnceLock::new());
    let seen = first.clone();
    let notify: mail::Notify = Arc::new(move |e| {
        if matches!(e, mail::Event::FolderChanged { .. }) {
            seen.get_or_init(|| ms(t));
        }
    });
    mail::sync_folder_notify(imap, db, f, &notify)?;
    match first.get() {
        Some(&ms) => {
            let s = Sample {
                ms,
                bytes: None,
                cmds: None,
            };
            report.add("inbox first rows", s, "  first batch inserted");
        }
        None => println!("inbox first rows: none, empty inbox"),
    }
    Ok(())
}

/// A changed inbox, simulated in the throwaway cache only (the server is
/// just read, with EXAMINE): the 5 newest rows are dropped and the
/// fingerprint set back to before them, with the modseq one lower. Synced
/// once on the full path, then again on the delta, as a steady-state pass
/// would after a STATUS mismatch.
fn changed_inbox(
    imap: &mut Imap,
    db: &mut Db,
    account_id: i64,
    inbox: &Folder,
    report: &mut Report,
) -> Result<()> {
    let (_, statuses) = mail::refresh_folders(imap, db, account_id)?;
    let Some(st) = statuses.get(&inbox.name).filter(|st| st.hms > 0) else {
        println!("delta inbox (simulated): skipped, no HIGHESTMODSEQ");
        return Ok(());
    };
    for delta in [false, true] {
        let mut uids: Vec<u32> = db.known_uids(inbox.id)?.into_iter().collect();
        uids.sort_unstable_by(|a, b| b.cmp(a));
        uids.truncate(5);
        let Some(&lowest) = uids.last() else {
            println!("delta inbox (simulated): skipped, empty inbox");
            return Ok(());
        };
        db.delete_uids(inbox.id, &uids)?;
        let fp = Fp {
            uidnext: lowest,
            messages: db.count_messages(inbox.id)?,
            hms: st.hms - 1,
        };
        db.finish_folder(inbox.id, fp)?;
        let m = Mark::new(imap);
        let got = if delta {
            mail::sync_folder_with_status(imap, db, inbox, st)?
        } else {
            mail::sync_folder_once(imap, db, inbox)?
        };
        let (label, note) = match (delta, got.delta) {
            (false, _) => ("changed inbox full (simulated)", ""),
            (true, true) => ("delta inbox (simulated)", ""),
            (true, false) => ("delta inbox (simulated)", ", took the full path"),
        };
        report.add(
            label,
            m.sample(imap),
            &format!("  {} new, {} flag changes{note}", got.new, got.flag_changes),
        );
    }
    Ok(())
}

/// TCP connect, TLS, greeting, LOGIN and CAPABILITY timed separately, on a
/// connection of its own that is logged out right after.
fn connect_phases(
    server: &Server,
    username: &str,
    password: &str,
    report: &mut Report,
) -> Result<()> {
    if server.security != Security::Tls {
        println!("connect phases: skipped (STARTTLS)");
        return Ok(());
    }
    let phase = |t: Instant, cmds| Sample {
        ms: ms(t),
        bytes: None,
        cmds: Some(cmds),
    };
    let t = Instant::now();
    let addr = (server.host.as_str(), server.port)
        .to_socket_addrs()?
        .next()
        .context("no address")?;
    report.add("connect: dns", phase(t, 0), "");
    let t = Instant::now();
    let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(15))?;
    tcp.set_read_timeout(Some(Duration::from_secs(90)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(90)))?;
    report.add("connect: tcp", phase(t, 0), "");
    let t = Instant::now();
    let tls = TlsConnector::new()?
        .connect(&server.host, tcp)
        .map_err(|e| anyhow!("TLS handshake failed: {e}"))?;
    report.add("connect: tls", phase(t, 0), "");
    let mut client = imap::Client::new(tls);
    let t = Instant::now();
    client.read_greeting()?;
    report.add("connect: greeting", phase(t, 0), "");
    let t = Instant::now();
    let mut session = client
        .login(username, password)
        .map_err(|(e, _)| anyhow!("sign-in failed: {e}"))?;
    report.add("connect: login", phase(t, 1), "");
    let t = Instant::now();
    session.capabilities()?;
    report.add("connect: capability", phase(t, 1), "");
    session.logout().ok();
    Ok(())
}

/// Sizes of the newest inbox messages (the prefetch window), read from the
/// throwaway cache: count and total bytes at or under each cutoff.
fn size_histogram(db_path: &str, folder_id: i64) -> Result<()> {
    let conn = rusqlite::Connection::open(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT size FROM messages WHERE folder_id = ?1
         ORDER BY date DESC, id DESC LIMIT ?2",
    )?;
    let sizes: Vec<i64> = stmt
        .query_map(
            rusqlite::params![folder_id, mail::PREFETCH.window as i64],
            |r| r.get(0),
        )?
        .collect::<rusqlite::Result<_>>()?;
    println!("size histogram, newest {} inbox msgs:", sizes.len());
    for kib in [64, 128, 256, 512] {
        let fit = sizes.iter().filter(|&&s| s <= kib * 1024);
        println!(
            "  <= {kib:>3}K: {:>4} msgs, {:>10} B",
            fit.clone().count(),
            fit.sum::<i64>()
        );
    }
    println!(
        "  all     : {:>4} msgs, {:>10} B",
        sizes.len(),
        sizes.iter().sum::<i64>()
    );
    Ok(())
}

fn print_capabilities(caps: &[String]) {
    println!("capabilities: {}", caps.join(" "));
    for want in [
        "IDLE",
        "CONDSTORE",
        "QRESYNC",
        "ENABLE",
        "COMPRESS=DEFLATE",
        "MOVE",
        "UIDPLUS",
        "LITERAL+",
        "ESEARCH",
        "SPECIAL-USE",
        "LIST-STATUS",
        "SORT",
        "THREAD=REFERENCES",
        "NOTIFY",
        "OBJECTID",
        "STATUS=SIZE",
        "UTF8=ACCEPT",
        "IMAP4rev2",
    ] {
        println!(
            "  {want}: {}",
            caps.iter().any(|c| c.eq_ignore_ascii_case(want))
        );
    }
}

/// One measured line: wall time, IMAP payload bytes and commands (`None`
/// where the probe can't count them).
#[derive(Clone, Copy, Default)]
struct Sample {
    ms: f64,
    bytes: Option<u64>,
    cmds: Option<u64>,
}

impl Sample {
    /// Local work only: no bytes, no commands.
    fn local(t: Instant) -> Self {
        Self {
            ms: ms(t),
            bytes: Some(0),
            cmds: Some(0),
        }
    }

    fn add(&mut self, other: Sample) {
        let sum = |a: Option<u64>, b: Option<u64>| Some(a.unwrap_or(0) + b?);
        self.ms += other.ms;
        self.bytes = sum(self.bytes, other.bytes);
        self.cmds = sum(self.cmds, other.cmds);
    }
}

/// Where a measurement started, for byte and command deltas.
struct Mark {
    t: Instant,
    bytes: u64,
    cmds: u64,
}

impl Mark {
    fn new(imap: &Imap) -> Self {
        Self {
            t: Instant::now(),
            bytes: imap.bytes(),
            cmds: imap.commands(),
        }
    }

    fn sample(&self, imap: &Imap) -> Sample {
        Sample {
            ms: ms(self.t),
            bytes: Some(imap.bytes() - self.bytes),
            cmds: Some(imap.commands() - self.cmds),
        }
    }
}

/// Every line printed so far, keyed by label, so runs can be summarized.
#[derive(Default)]
struct Report {
    lines: Vec<(String, Vec<Sample>)>,
}

impl Report {
    fn add(&mut self, label: &str, s: Sample, note: &str) {
        println!("{}{note}", line(label, s));
        match self.lines.iter_mut().find(|(l, _)| l == label) {
            Some((_, samples)) => samples.push(s),
            None => self.lines.push((label.to_string(), vec![s])),
        }
    }

    fn print_medians(&self) {
        for (label, samples) in &self.lines {
            let median = |get: fn(&Sample) -> Option<u64>| {
                let v: Option<Vec<f64>> =
                    samples.iter().map(|s| get(s).map(|n| n as f64)).collect();
                v.map(|v| percentile(&v, 0.5) as u64)
            };
            let ms: Vec<f64> = samples.iter().map(|s| s.ms).collect();
            let s = Sample {
                ms: percentile(&ms, 0.5),
                bytes: median(|s| s.bytes),
                cmds: median(|s| s.cmds),
            };
            println!("{}", line(label, s));
        }
    }
}

fn line(label: &str, s: Sample) -> String {
    let opt = |v: Option<u64>| v.map_or("-".to_string(), |n| n.to_string());
    format!(
        "  {label:<34} {:>9.1} ms  {:>10} B  {:>5} cmds",
        s.ms,
        opt(s.bytes),
        opt(s.cmds)
    )
}

/// Nearest-rank percentile (`p` in 0..=1); the median of an even count is
/// the lower middle value.
fn percentile(values: &[f64], p: f64) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let rank = (v.len() as f64 * p).ceil() as usize;
    v[rank.clamp(1, v.len()) - 1]
}

fn label(role: Role, i: usize) -> String {
    match role {
        Role::Other => format!("folder#{i}"),
        r => r.key().to_string(),
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn chrono_free_stamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Each run deletes the cache file, so it must be a throwaway one: never a
/// file in the app's cache directory.
fn refuse_real_cache(path: &str) -> Result<()> {
    let lower = |p: &std::path::Path| -> Result<String> {
        Ok(std::path::absolute(p)?.to_string_lossy().to_lowercase())
    };
    if let Some(dir) = dirs::data_local_dir()
        && lower(path.as_ref())?.starts_with(&lower(&dir.join("komorebi"))?)
    {
        bail!("refusing to use the app's cache directory: give a throwaway path");
    }
    Ok(())
}

fn read_env(path: &str) -> Result<HashMap<String, String>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
    Ok(text
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| {
            (
                k.trim().to_string(),
                v.trim().trim_matches(['"', '\'']).to_string(),
            )
        })
        .collect())
}
