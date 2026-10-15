//! The IDLE thread: a read-only connection in IMAP IDLE on the inbox that
//! only reports changes ([`Event::RemoteChanged`]); the sync worker fetches
//! them. It notifies only: no SQLite, no job queue, so dropping the
//! [`super::Workers`] still closes the job channels.

use super::errors::{Latch, is_parse_error, is_refusal};
use super::imap::{Imap, WaitOutcome};
use super::{Connect, Event, Notify};
use crate::db::Account;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One IDLE lasts at most this long before it is renewed (RFC 2177: under
/// 29 minutes; shorter outlives typical NAT idle timeouts).
const IDLE_WAIT: Duration = Duration::from_secs(9 * 60);
/// Changes come in bursts (a message, then its flags): wait this long after
/// a wake before asking for a sync.
const IDLE_DEBOUNCE: Duration = Duration::from_millis(300);
/// First wait after a failed IDLE connection; it doubles up to the cap.
const IDLE_BACKOFF: Duration = Duration::from_secs(5);
const IDLE_BACKOFF_MAX: Duration = Duration::from_secs(10 * 60);
/// A connection that stayed up this long resets the backoff.
const IDLE_HEALTHY: Duration = Duration::from_secs(60);
/// Sign-in refusals in a row (`[UNAVAILABLE]`, connection limits) after
/// which the thread gives up; the 1-minute pass remains.
const IDLE_REFUSALS: u32 = 3;
/// How often `stop` is checked while waiting for the first sync.
const IDLE_SLICE: Duration = Duration::from_millis(200);

/// Delivered messages in a row, found by a steady-state pass while an IDLE
/// connection was up and silent, after which the server is taken to never
/// announce new mail over IDLE (Dovecot on Postale.io reports only flag
/// changes). Conservative on purpose: five misses, no hit in between.
pub(super) const IDLE_SILENT_AFTER: u32 = 5;

/// Whether IDLE announces newly delivered mail on this server: a pure state
/// machine fed by the sync worker after each inbox pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct IdleHealth {
    missed: u32,
    last_session: Option<u64>,
    silent: bool,
}

impl IdleHealth {
    /// One steady-state inbox pass found `found` new messages. `woke`:
    /// IDLE woke the sync since the previous pass. `session`: the live IDLE
    /// connection, if any. Counts a miss only when the same connection was
    /// up through the whole interval (mail that arrived while IDLE was
    /// down or reconnecting proves nothing), a hit (any wake) resets the
    /// count, and a pass that found nothing changes nothing.
    pub(super) fn observe(&mut self, found: usize, woke: bool, session: Option<u64>) {
        let steady = session.is_some() && session == self.last_session;
        self.last_session = session;
        if self.silent || found == 0 || !steady {
            return;
        }
        if woke {
            self.missed = 0;
        } else {
            self.missed = self.missed.saturating_add(1);
            self.silent = self.missed >= IDLE_SILENT_AFTER;
        }
    }

    /// IDLE is judged useless for new mail on this server (until restart).
    pub(super) fn silent(&self) -> bool {
        self.silent
    }
}

/// What the IDLE thread and the sync worker tell each other: wakes, the live
/// connection and the verdict of [`IdleHealth`].
#[derive(Debug, Default)]
pub(super) struct IdleWatch {
    woke: AtomicBool,
    /// The live IDLE connection's number; 0 when none.
    session: AtomicU64,
    next: AtomicU64,
    health: Mutex<IdleHealth>,
}

impl IdleWatch {
    pub(super) fn started(&self) {
        let id = self.next.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        self.session.store(id, Ordering::Relaxed);
    }

    pub(super) fn ended(&self) {
        self.session.store(0, Ordering::Relaxed);
    }

    pub(super) fn woke(&self) {
        self.woke.store(true, Ordering::Relaxed);
    }

    /// An inbox pass (steady state) found `found` new messages.
    pub(super) fn observe(&self, found: usize) {
        let woke = self.woke.swap(false, Ordering::Relaxed);
        let session = Some(self.session.load(Ordering::Relaxed)).filter(|&s| s != 0);
        if let Ok(mut h) = self.health.lock() {
            h.observe(found, woke, session);
        }
    }

    pub(super) fn silent(&self) -> bool {
        self.health.lock().is_ok_and(|h| h.silent())
    }
}

/// The IDLE thread's state.
pub(super) struct Idler {
    account: Account,
    connect: Connect,
    notify: Notify,
    auth_failed: Latch,
    stop: Arc<AtomicBool>,
    /// Set by [`IdleHandle::kick`]: reconnect now, with the backoff reset.
    kicked: Arc<AtomicBool>,
    /// The live connection's socket, for `shutdown` from [`IdleHandle`].
    sock: Arc<Mutex<Option<std::net::TcpStream>>>,
    /// How long one IDLE runs ([`IDLE_WAIT`]; short in tests).
    wait: Duration,
    /// Shared with the sync worker: wakes in, the silent-server verdict out.
    watch: Arc<IdleWatch>,
}

/// How one IDLE connection ended.
enum IdleEnd {
    /// Asked to stop.
    Stopped,
    /// Never try again: no IDLE, a NO/BAD to IDLE, or a reply the stream
    /// refused.
    Quit,
    /// The server refused the sign-in for now (e.g. too many connections).
    Refused,
    /// The connection failed; reconnect after the backoff.
    Retry,
}

/// Stops the IDLE thread when dropped: sets `stop`, shuts its socket down
/// (ending a blocked read) and wakes it from a backoff. Never joins.
pub(super) struct IdleHandle {
    stop: Arc<AtomicBool>,
    kicked: Arc<AtomicBool>,
    sock: Arc<Mutex<Option<std::net::TcpStream>>>,
    thread: std::thread::JoinHandle<()>,
}

impl IdleHandle {
    fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.cut();
    }

    /// After a wake from sleep: drop the (likely dead) connection and
    /// open a new one now, whatever the backoff was.
    pub(super) fn kick(&self) {
        self.kicked.store(true, Ordering::Relaxed);
        self.cut();
    }

    /// Shut the live socket down (ending a blocked read) and wake the
    /// thread from a nap.
    fn cut(&self) {
        if let Some(sock) = self.sock.lock().expect("idle socket lock").as_ref() {
            sock.shutdown(std::net::Shutdown::Both).ok();
        }
        self.thread.thread().unpark();
    }
}

impl Drop for IdleHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Start the IDLE thread. It waits for the inbox from `ready`, which the
/// sync worker sends after the inbox first synced.
pub(super) fn spawn_idle(idler: Idler, ready: Receiver<(i64, String)>) -> IdleHandle {
    let (stop, kicked, sock) = (idler.stop.clone(), idler.kicked.clone(), idler.sock.clone());
    let thread = std::thread::Builder::new()
        .name(format!("idle-{}", idler.account.id))
        .spawn(move || idler.run(&ready))
        .expect("spawning the IDLE thread");
    IdleHandle {
        stop,
        kicked,
        sock,
        thread,
    }
}

impl Idler {
    pub(super) fn new(
        account: Account,
        connect: Connect,
        notify: Notify,
        auth_failed: Latch,
    ) -> Self {
        Self {
            account,
            connect,
            notify,
            auth_failed,
            stop: Arc::default(),
            kicked: Arc::default(),
            sock: Arc::default(),
            wait: IDLE_WAIT,
            watch: Arc::default(),
        }
    }

    /// Share `watch` with the sync worker, which feeds it the inbox passes.
    pub(super) fn with_watch(mut self, watch: Arc<IdleWatch>) -> Self {
        self.watch = watch;
        self
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Sleep for `d` unless stopped (false) or kicked first.
    fn nap(&self, d: Duration) -> bool {
        let until = Instant::now() + d;
        while !self.stopped() {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() || self.kicked.load(Ordering::Relaxed) {
                return true;
            }
            std::thread::park_timeout(left);
        }
        false
    }

    fn run(&self, ready: &Receiver<(i64, String)>) {
        let inbox = loop {
            if self.stopped() {
                return;
            }
            match ready.recv_timeout(IDLE_SLICE) {
                Ok(inbox) => break inbox,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
        };
        let mut backoff = IDLE_BACKOFF;
        let mut refusals = 0;
        let mut rng = seed();
        for attempt in 0.. {
            // A rejected sign-in waits for the user to retry (Refresh).
            while self.auth_failed.get() {
                // A kick has no connection to cut here (and would cut
                // every nap short).
                self.kicked.store(false, Ordering::Relaxed);
                if !self.nap(Duration::from_secs(1)) {
                    return;
                }
            }
            if self.stopped() {
                return;
            }
            let started = Instant::now();
            let end = self.watch(&inbox, attempt > 0);
            // The clone would keep the dropped connection open.
            self.sock.lock().expect("idle socket lock").take();
            match end {
                IdleEnd::Stopped | IdleEnd::Quit => return,
                IdleEnd::Refused => {
                    refusals += 1;
                    if refusals >= IDLE_REFUSALS {
                        return;
                    }
                }
                IdleEnd::Retry => refusals = 0,
            }
            if started.elapsed() >= IDLE_HEALTHY {
                backoff = IDLE_BACKOFF;
            }
            if !self.kicked.load(Ordering::Relaxed) {
                if !self.nap(jitter(backoff, &mut rng)) {
                    return;
                }
                backoff = (backoff * 2).min(IDLE_BACKOFF_MAX);
            }
            // Woken from sleep: reconnect at once, starting the backoff over.
            if self.kicked.swap(false, Ordering::Relaxed) {
                backoff = IDLE_BACKOFF;
                refusals = 0;
            }
        }
    }

    /// One connection: sign in, EXAMINE the inbox and IDLE until something
    /// fails or the thread is stopped. `catch_up` reports a change first, for
    /// whatever arrived while disconnected.
    fn watch(&self, (inbox_id, inbox): &(i64, String), catch_up: bool) -> IdleEnd {
        let mut imap = match (self.connect)(&self.account) {
            Ok(imap) => imap,
            // Shared with the other workers, so none of them adds a failed
            // login (or another failed handshake) either.
            Err(e) if self.auth_failed.set_from(&e) => return IdleEnd::Retry,
            Err(e) if is_refusal(&e) => return IdleEnd::Refused,
            Err(_) => return IdleEnd::Retry,
        };
        *self.sock.lock().expect("idle socket lock") = imap.socket().try_clone().ok();
        // Stopped before the socket was stored: nothing shut it down.
        if self.stopped() {
            return IdleEnd::Stopped;
        }
        // Without IDLE the server would refuse it; don't ask.
        if !imap.has_capability("IDLE") {
            imap.logout();
            return IdleEnd::Quit;
        }
        imap.set_read_only(true);
        if imap.select(inbox).is_err() {
            return IdleEnd::Retry;
        }
        imap.drain_unsolicited();
        let changed = || {
            (self.notify)(Event::RemoteChanged {
                folder_id: *inbox_id,
            })
        };
        if catch_up {
            changed();
        }
        self.watch.started();
        let end = self.idle_loop(imap, changed);
        self.watch.ended();
        end
    }

    fn idle_loop(&self, mut imap: Imap, changed: impl Fn()) -> IdleEnd {
        loop {
            match imap.idle(self.wait) {
                Ok(WaitOutcome::MailboxChanged) => {
                    if !self.nap(IDLE_DEBOUNCE) {
                        return IdleEnd::Stopped;
                    }
                    self.watch.woke();
                    changed();
                }
                Ok(WaitOutcome::TimedOut) => {
                    // The server never announced delivered mail over this
                    // connection: give the connection back (the timer's
                    // passes find new mail, as for a server without IDLE).
                    if self.watch.silent() {
                        imap.logout();
                        return IdleEnd::Quit;
                    }
                }
                // A shutdown from `IdleHandle` ends the read with an error.
                Err(_) if self.stopped() => return IdleEnd::Stopped,
                // A tagged NO or BAD: no IDLE here; or a reply the stream
                // refused (a tagged OK without `+`). The connection is
                // dropped, not logged out (see `Imap::idle`).
                Err(e) if is_refusal(&e) || is_parse_error(&e) => return IdleEnd::Quit,
                Err(_) => return IdleEnd::Retry,
            }
            if self.stopped() {
                return IdleEnd::Stopped;
            }
        }
    }
}

/// A seed for [`jitter`], from the clock (never 0).
fn seed() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    nanos | 1
}

/// `d` scaled by a random factor in 0.75..1.25 (xorshift), so accounts on
/// one server don't reconnect in step.
fn jitter(d: Duration, state: &mut u64) -> Duration {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    d.mul_f64(0.75 + (*state % 1000) as f64 / 2000.0)
}

#[cfg(test)]
mod tests {
    use super::super::fake::*;
    use super::super::imap::Imap;
    use super::super::tests::setup;
    use super::*;
    use crate::db::Db;
    use std::sync::mpsc::channel;

    const IDLE_CAPS: &str = "* CAPABILITY IMAP4rev1 IDLE\r\n{tag} OK done\r\n";

    /// An IDLE thread on `connect`, its inbox (id 1, "INBOX") already
    /// handed over; returns its handle and the events it sends. The thread
    /// holds the only sender, so the receiver disconnects once it exits.
    fn idler(db: &Db, connect: Connect) -> (IdleHandle, Receiver<Event>) {
        idler_with(db, connect, Latch::default())
    }

    /// [`idler`] sharing `latch` with the test, as the account's other
    /// workers share it.
    fn idler_with(db: &Db, connect: Connect, latch: Latch) -> (IdleHandle, Receiver<Event>) {
        let (events, rx) = channel();
        let events = Mutex::new(events);
        let (ready, ready_rx) = channel();
        ready.send((1, "INBOX".to_string())).unwrap();
        let mut idler = Idler::new(
            db.accounts().unwrap().remove(0),
            connect,
            Arc::new(move |e| {
                events.lock().unwrap().send(e).ok();
            }),
            latch,
        );
        idler.wait = Duration::from_millis(300);
        (spawn_idle(idler, ready_rx), rx)
    }

    fn connect_plain(addr: std::net::SocketAddr) -> Connect {
        Box::new(move |_| Imap::connect_plain(addr, "user", "pass"))
    }

    /// Whether the IDLE thread behind `events` exits within `limit`
    /// without sending anything more.
    fn exits_within(events: &Receiver<Event>, limit: Duration) -> bool {
        matches!(
            events.recv_timeout(limit),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        )
    }

    #[test]
    fn idle_thread_reports_exists_but_not_keepalives() {
        let mut db = Db::in_memory().unwrap();
        setup(&mut db, 7, &[]);
        let (release, held) = channel::<()>();
        let (renewed, idling) = channel();
        let mut script = login_steps(IDLE_CAPS);
        script.extend([
            Step::Expect(r#"a3 EXAMINE "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 IDLE"),
            Step::Send("+ idling\r\n* OK Still here\r\n"),
            // The short wait ends: DONE, then a new IDLE.
            Step::Expect("DONE"),
            Step::Send("a4 OK IDLE terminated\r\n"),
            Step::Expect("a5 IDLE"),
            Step::Send("+ idling\r\n* 5 EXISTS\r\n"),
            Step::Expect("DONE"),
            Step::Send("a5 OK IDLE terminated\r\n"),
            Step::Expect("a6 IDLE"),
            Step::Send("+ idling\r\n"),
            // Hold the connection open until the test is done.
            Step::Then(Box::new(move || {
                renewed.send(()).unwrap();
                held.recv().ok();
            })),
        ]);
        let server = FakeServer::start(script);
        let (handle, events) = idler(&db, connect_plain(server.addr));

        // The first event is the EXISTS: the keepalive woke nothing.
        let first = events.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(first, Event::RemoteChanged { folder_id: 1 }));
        // Dropping the handle during an IDLE shuts the socket down.
        idling.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(handle);
        assert!(exits_within(&events, Duration::from_secs(1)));
        drop(release);
        let sent = server.finish();
        assert_eq!(sent.iter().filter(|l| *l == "DONE").count(), 2);
    }

    fn watch_after(passes: &[(usize, bool, Option<u64>)]) -> IdleHealth {
        let mut h = IdleHealth::default();
        for &(found, woke, session) in passes {
            h.observe(found, woke, session);
        }
        h
    }

    #[test]
    fn silent_idle_is_judged_only_after_repeated_misses() {
        let miss = (1, false, Some(1));
        // The first pass only learns the connection; then five misses.
        let mut h = IdleHealth::default();
        for i in 0..=IDLE_SILENT_AFTER {
            assert!(!h.silent(), "pass {i}");
            h.observe(miss.0, miss.1, miss.2);
        }
        assert!(h.silent());
        // Four misses are not enough.
        let four = vec![miss; IDLE_SILENT_AFTER as usize];
        assert!(!watch_after(&four).silent());
    }

    #[test]
    fn a_wake_a_quiet_pass_or_a_new_connection_never_counts_as_a_miss() {
        let miss = (1, false, Some(1));
        // A wake with new mail resets the count.
        let mut passes = vec![miss; 4];
        passes.push((1, true, Some(1)));
        passes.extend(vec![miss; 4]);
        assert!(!watch_after(&passes).silent());
        // Passes with nothing new change nothing.
        let mut passes = vec![miss; 4];
        passes.extend(vec![(0, false, Some(1)); 50]);
        passes.push(miss);
        passes.push(miss);
        assert!(watch_after(&passes).silent());
        // Mail found while IDLE was down, or on a connection that just
        // changed, proves nothing.
        assert!(!watch_after(&vec![(1, false, None); 20]).silent());
        let alternating: Vec<_> = (1..40).map(|i| (1, false, Some(i))).collect();
        assert!(!watch_after(&alternating).silent());
    }

    #[test]
    fn the_verdict_is_final_for_the_session() {
        let mut passes = vec![(1, false, Some(1)); 10];
        passes.extend(vec![(1, true, Some(1)); 10]);
        assert!(watch_after(&passes).silent());
    }

    #[test]
    fn the_watch_feeds_the_machine_from_wakes_and_connections() {
        let w = IdleWatch::default();
        w.started();
        w.observe(0);
        for _ in 0..IDLE_SILENT_AFTER {
            assert!(!w.silent());
            w.observe(2);
        }
        assert!(w.silent());
        // A wake is consumed by the next pass.
        let w = IdleWatch::default();
        w.started();
        w.observe(0);
        for _ in 0..20 {
            w.woke();
            w.observe(1);
        }
        assert!(!w.silent());
        // Without a connection nothing is judged.
        let w = IdleWatch::default();
        for _ in 0..20 {
            w.observe(1);
        }
        assert!(!w.silent());
    }

    #[test]
    fn a_silent_server_gets_its_idle_connection_back() {
        let mut db = Db::in_memory().unwrap();
        setup(&mut db, 7, &[]);
        let mut script = login_steps(IDLE_CAPS);
        script.extend([
            Step::Expect(r#"a3 EXAMINE "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 IDLE"),
            Step::Send("+ idling\r\n"),
            Step::Expect("DONE"),
            Step::Send("a4 OK IDLE terminated\r\n"),
        ]);
        script.extend(logout_steps("a5 LOGOUT"));
        let server = FakeServer::start(script);
        let (events, rx) = channel();
        let events = Mutex::new(events);
        let (ready, ready_rx) = channel();
        ready.send((1, "INBOX".to_string())).unwrap();
        let watch = Arc::new(IdleWatch::default());
        let mut idler = Idler::new(
            db.accounts().unwrap().remove(0),
            connect_plain(server.addr),
            Arc::new(move |e| {
                events.lock().unwrap().send(e).ok();
            }),
            Latch::default(),
        )
        .with_watch(watch.clone());
        idler.wait = Duration::from_millis(300);
        // The verdict is in before the first IDLE ends.
        *watch.health.lock().unwrap() = IdleHealth {
            silent: true,
            ..Default::default()
        };
        let _handle = spawn_idle(idler, ready_rx);
        // The thread logs out and is gone: no reconnect, no event.
        assert!(exits_within(&rx, Duration::from_secs(5)));
        server.finish();
    }

    #[test]
    fn idle_thread_quits_when_idle_is_refused() {
        let mut db = Db::in_memory().unwrap();
        setup(&mut db, 7, &[]);
        let mut script = login_steps(IDLE_CAPS);
        script.extend([
            Step::Expect(r#"a3 EXAMINE "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 IDLE"),
            Step::Send("{tag} NO IDLE not allowed\r\n"),
            Step::ExpectEof,
        ]);
        let server = FakeServer::start(script);
        let (_handle, events) = idler(&db, connect_plain(server.addr));

        // No reconnect, no event: the thread is gone with the handle alive.
        assert!(exits_within(&events, Duration::from_secs(5)));
        server.finish();
    }

    /// A tagged OK to IDLE without the `+` continuation would reach the
    /// crate's `unreachable!()` (an abort in release builds). The stream
    /// refuses it: the connection fails, no DONE goes out, and the thread
    /// quits (the 1-minute pass remains).
    #[test]
    fn idle_thread_quits_on_a_tagged_ok_without_continuation() {
        let mut db = Db::in_memory().unwrap();
        setup(&mut db, 7, &[]);
        let mut script = login_steps(IDLE_CAPS);
        script.extend([
            Step::Expect(r#"a3 EXAMINE "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 IDLE"),
            Step::Send("{tag} OK IDLE done\r\n"),
            Step::ExpectEof,
        ]);
        let server = FakeServer::start(script);
        let (_handle, events) = idler(&db, connect_plain(server.addr));

        assert!(exits_within(&events, Duration::from_secs(5)));
        assert!(server.finish().iter().all(|l| l != "DONE"));
    }

    #[test]
    fn dropping_the_idle_handle_cuts_the_backoff_short() {
        let mut db = Db::in_memory().unwrap();
        setup(&mut db, 7, &[]);
        let (tried, attempts) = channel();
        let tried = Mutex::new(tried);
        let (handle, events) = idler(
            &db,
            Box::new(move |_| {
                tried.lock().unwrap().send(()).ok();
                Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused).into())
            }),
        );
        attempts.recv_timeout(Duration::from_secs(5)).unwrap();
        // Now in its first backoff, about 5 s.
        std::thread::sleep(Duration::from_millis(100));
        drop(handle);
        assert!(exits_within(&events, Duration::from_secs(1)));
        assert!(attempts.try_recv().is_err(), "no second attempt");
    }

    /// After a wake from sleep (`Workers::resumed`), the thread reconnects
    /// at once instead of finishing its backoff.
    #[test]
    fn a_kick_reconnects_without_waiting_out_the_backoff() {
        let mut db = Db::in_memory().unwrap();
        setup(&mut db, 7, &[]);
        let (tried, attempts) = channel();
        let tried = Mutex::new(tried);
        let (handle, _events) = idler(
            &db,
            Box::new(move |_| {
                tried.lock().unwrap().send(()).ok();
                Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused).into())
            }),
        );
        attempts.recv_timeout(Duration::from_secs(5)).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        handle.kick();
        // The second attempt comes well inside the ~5 s backoff.
        attempts.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(handle);
    }

    #[test]
    fn idle_thread_quits_on_a_server_without_idle() {
        // The profile of a server with no IDLE: the thread logs out and is
        // gone; the poll timer finds new mail instead.
        let mut db = Db::in_memory().unwrap();
        setup(&mut db, 7, &[]);
        let mut script = login_steps(PLAIN_CAPS);
        script.extend(logout_steps("a3 LOGOUT"));
        let server = FakeServer::start(script);
        let (_handle, events) = idler(&db, connect_plain(server.addr));

        assert!(exits_within(&events, Duration::from_secs(5)));
        server.finish();
    }

    /// A changed certificate or a rejected password is one attempt: the
    /// thread latches the account (shared with its other workers) instead of
    /// asking the server again after every backoff.
    #[test]
    fn idle_thread_latches_the_account_on_a_failure_only_the_user_can_fix() {
        type Fail = fn() -> anyhow::Error;
        let cases: [(Fail, bool); 3] = [
            (
                || crate::mail::pin::PinMismatch("imap.example.com".into()).into(),
                true,
            ),
            (
                || crate::mail::imap::TlsFailed("imap.example.com".into()).into(),
                true,
            ),
            (|| crate::mail::imap::AuthFailed("no".into()).into(), false),
        ];
        for (fail, secure) in cases {
            let mut db = Db::in_memory().unwrap();
            setup(&mut db, 7, &[]);
            let latch = Latch::default();
            let tries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let count = tries.clone();
            let (handle, _events) = idler_with(
                &db,
                Box::new(move |_| {
                    count.fetch_add(1, Ordering::SeqCst);
                    Err(fail())
                }),
                latch.clone(),
            );
            let until = Instant::now() + Duration::from_secs(5);
            while !latch.get() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(latch.get(), "latched");
            assert_eq!(latch.is_secure(), secure);
            assert_eq!(tries.load(Ordering::SeqCst), 1);
            drop(handle);
        }
        // A refused connection is not such a failure.
        let latch = Latch::default();
        assert!(
            !latch.set_from(&std::io::Error::from(std::io::ErrorKind::ConnectionRefused).into())
        );
        assert!(!latch.get());
    }
}
