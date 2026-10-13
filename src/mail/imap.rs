//! Thin IMAP session wrapper (blocking, used from worker threads only).
//!
//! This is the only file that names the `imap` crate (an abandoned parser):
//! every command goes through [`Imap::run`], failures leave as [`Failure`],
//! and the stream ([`Counted`]) refuses the replies the crate would abort
//! on (a tagged reply with another command's tag, a tagged OK to IDLE).

use super::errors::Step as CheckStep;
use super::mime::{HEADER_FIELDS, parse_header};
use crate::db::{self, NewHeader};
use crate::diag;
use crate::providers::{Security, Server};
use crate::role::Role;
use anyhow::{Context, Result, anyhow};
use imap::extensions::idle::SetReadTimeout;
use imap::types::{Flag, Name, NameAttribute, StatusAttribute, UnsolicitedResponse};
use imap_proto::types::Capability;
use native_tls::TlsStream;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

/// How an IDLE wait ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    TimedOut,
    MailboxChanged,
}

/// What an IMAP command's failure was, for callers that must not name the
/// crate's error type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The connection failed or was lost (I/O, EOF, a timeout).
    Unreachable,
    /// A reply the parser can't read, or one the stream refused: the
    /// stream is out of step and the command may have run.
    Protocol,
    /// A tagged NO, with its text (a response code first, if any).
    No(String),
    /// A tagged BAD, with its text.
    Bad(String),
}

/// The [`Failure`] in `e`'s chain, if an IMAP command failed.
pub fn failure(e: &anyhow::Error) -> Option<Failure> {
    e.chain().find_map(|c| {
        Some(match c.downcast_ref::<imap::Error>()? {
            imap::Error::Io(io) if violation(io) => Failure::Protocol,
            imap::Error::Io(_) | imap::Error::ConnectionLost => Failure::Unreachable,
            imap::Error::No(text) => Failure::No(text.clone()),
            imap::Error::Bad(text) => Failure::Bad(text.clone()),
            // APPEND refused before its literal (the crate drops the text);
            // the stream is in step.
            imap::Error::Append => Failure::No(String::new()),
            // Refused before anything was sent.
            imap::Error::Validate(v) => Failure::Bad(v.to_string()),
            _ => Failure::Protocol,
        })
    })
}

/// The crate's own description of an IMAP failure in `e` (its Debug form
/// names the response it rejected), for development tools.
pub fn detail(e: &anyhow::Error) -> Option<String> {
    e.chain()
        .find_map(|c| c.downcast_ref::<imap::Error>())
        .map(|ie| format!("{ie:?}"))
}

/// A server reply the stream refused to hand to the crate.
#[derive(Debug)]
struct Violation(&'static str);

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "the server sent {}", self.0)
    }
}

impl std::error::Error for Violation {}

/// Whether an I/O error is a refused reply ([`Violation`]), not the network.
fn violation(e: &std::io::Error) -> bool {
    e.get_ref().is_some_and(|inner| inner.is::<Violation>())
}

/// A stream that counts the bytes it carries, so sync cost can be measured
/// (TLS payload bytes, i.e. IMAP protocol traffic). It also follows the
/// lines both ways ([`Lines`]) to keep two replies from the crate:
/// - a tagged reply whose tag is not the last command's: the crate asserts
///   on it, which aborts a release build;
/// - anything but `+` or a tagged NO/BAD as the first reply to IDLE: on a
///   tagged OK the crate reaches `unreachable!()`.
///
/// Either fails the read instead, so the connection is dropped.
pub struct Counted<S> {
    inner: S,
    bytes: Arc<AtomicU64>,
    done: Arc<DoneGuard>,
    sent: Lines,
    received: Lines,
    /// The tag of the last command sent.
    tag: Vec<u8>,
    /// IDLE was sent and its first reply has not arrived yet.
    idle_reply: bool,
}

impl<S> Counted<S> {
    fn new(inner: S, bytes: Arc<AtomicU64>, done: Arc<DoneGuard>) -> Self {
        Self {
            inner,
            bytes,
            done,
            // The client sends a literal only after the server's `+`.
            sent: Lines {
                hold: true,
                ..Lines::default()
            },
            received: Lines::default(),
            tag: Vec::new(),
            idle_reply: false,
        }
    }
}

/// The start of a line, enough for its tag and command.
const LINE_HEAD: usize = 32;
/// The end of a line kept, enough for a `{n}` literal announcement.
const LINE_TAIL: usize = 24;

/// Follows an IMAP byte stream line by line, passing over literals, so
/// each new line's start can be checked before the crate sees it.
#[derive(Default)]
struct Lines {
    /// Literal bytes still to pass over.
    literal: u64,
    /// The current line goes on after a literal: not a new line.
    continued: bool,
    head: Vec<u8>,
    tail: Vec<u8>,
    /// A literal announced at the end of a line only follows once the
    /// other side says so (the client's literals wait for a `+`).
    hold: bool,
    /// The literal announced and held, until [`Lines::release`].
    waiting: u64,
}

impl Lines {
    /// Follow `buf`, calling `line` with the start of every new line it
    /// completes (the crate acts on a line only once its LF has arrived).
    fn feed(
        &mut self,
        mut buf: &[u8],
        mut line: impl FnMut(&[u8]) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        while !buf.is_empty() {
            if self.literal > 0 {
                let n = usize::try_from(self.literal).map_or(buf.len(), |l| l.min(buf.len()));
                self.literal -= n as u64;
                buf = &buf[n..];
                continue;
            }
            let end = buf.iter().position(|&b| b == b'\n');
            let part = &buf[..end.map_or(buf.len(), |i| i + 1)];
            buf = &buf[part.len()..];
            if !self.continued && self.head.len() < LINE_HEAD {
                let take = (LINE_HEAD - self.head.len()).min(part.len());
                self.head.extend_from_slice(&part[..take]);
            }
            self.tail.extend_from_slice(part);
            if self.tail.len() > LINE_TAIL {
                self.tail.drain(..self.tail.len() - LINE_TAIL);
            }
            if end.is_some() {
                let new_line = !self.continued;
                let literal = announced(&self.tail);
                if self.hold {
                    (self.literal, self.waiting) = (0, literal);
                } else {
                    self.literal = literal;
                }
                self.continued = self.literal > 0;
                self.tail.clear();
                let head = std::mem::take(&mut self.head);
                if new_line {
                    line(&head)?;
                }
            }
        }
        Ok(())
    }
}

impl Lines {
    /// The other side asked for the held literal (`+`): pass over it, and
    /// take what follows it as the rest of its line.
    fn release(&mut self) {
        self.literal = std::mem::take(&mut self.waiting);
        self.continued = self.literal > 0;
    }
}

/// The size of the literal a line ending in `{n}` (or `{n+}`) announces;
/// 0 for none.
fn announced(line: &[u8]) -> u64 {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let Some(rest) = line.strip_suffix(b"}") else {
        return 0;
    };
    let rest = rest.strip_suffix(b"+").unwrap_or(rest);
    let digits = rest.iter().rev().take_while(|b| b.is_ascii_digit()).count();
    let Some(open) = rest.len().checked_sub(digits + 1) else {
        return 0;
    };
    if digits == 0 || rest[open] != b'{' {
        return 0;
    }
    std::str::from_utf8(&rest[open + 1..])
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(u64::MAX)
}

/// The first word of a line.
fn first_word(line: &[u8]) -> &[u8] {
    line.split(|&b| matches!(b, b' ' | b'\r' | b'\n'))
        .next()
        .unwrap_or_default()
}

/// Check the start of a reply line against the last command's tag (and,
/// right after IDLE, that it is the continuation or a refusal).
fn check_reply(head: &[u8], tag: &[u8], idle_reply: &mut bool) -> std::io::Result<()> {
    let refuse = |what| {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            Violation(what),
        ))
    };
    let word = first_word(head);
    if std::mem::take(idle_reply) {
        let status = first_word(head.get(word.len() + 1..).unwrap_or_default());
        let refusal = word == tag
            && (status.eq_ignore_ascii_case(b"NO") || status.eq_ignore_ascii_case(b"BAD"));
        if word != b"+" && !refusal {
            return refuse("no continuation for IDLE");
        }
    }
    if word == b"*" || word == b"+" || word == tag {
        Ok(())
    } else {
        refuse("a reply with an unexpected tag")
    }
}

/// Watches the DONE that ends an IDLE. The crate sends it from a `Drop`
/// that ignores every error, even after the server refused the IDLE.
#[derive(Default)]
struct DoneGuard {
    /// Refuse to send DONE. Set while IDLE awaits its continuation, and for
    /// good once the server refused: a reply to a stray DONE (Dovecot's
    /// `DONE BAD ...`) trips the crate's tag assert, which aborts a release
    /// build.
    refuse: AtomicBool,
    /// I/O failed since the last DONE went out, so its reply may be unread
    /// and the stream out of step.
    failed: AtomicBool,
    /// When the current IDLE wait must end. The crate's own timeout applies
    /// per read and restarts on every `* OK Still here` keepalive (Dovecot
    /// sends one every 2 minutes), so a longer wait would never end.
    deadline: std::sync::Mutex<Option<Instant>>,
}

impl Read for Counted<Transport> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let deadline = *self.done.deadline.lock().expect("deadline lock");
        if let Some(deadline) = deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                self.done.failed.store(true, Ordering::Relaxed);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "IDLE wait over",
                ));
            }
            self.inner.sock().set_read_timeout(Some(left))?;
        }
        let n = self
            .inner
            .read(buf)
            .inspect_err(|_| self.done.failed.store(true, Ordering::Relaxed))?;
        self.bytes.fetch_add(n as u64, Ordering::Relaxed);
        let Counted {
            received,
            sent,
            tag,
            idle_reply,
            ..
        } = self;
        received
            .feed(&buf[..n], |head| {
                // A literal the client announced goes out only after `+`;
                // a tagged reply instead refused it.
                match first_word(head) {
                    b"+" => sent.release(),
                    b"*" => {}
                    _ => sent.waiting = 0,
                }
                check_reply(head, tag, idle_reply)
            })
            .inspect_err(|_| self.done.failed.store(true, Ordering::Relaxed))?;
        Ok(n)
    }
}

impl<S: Write> Write for Counted<S> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.starts_with(b"DONE") {
            if self.done.refuse.load(Ordering::Relaxed) {
                return Err(std::io::Error::other("not ending a refused IDLE"));
            }
            self.done.failed.store(false, Ordering::Relaxed);
        }
        let n = self
            .inner
            .write(buf)
            .inspect_err(|_| self.done.failed.store(true, Ordering::Relaxed))?;
        self.bytes.fetch_add(n as u64, Ordering::Relaxed);
        let Counted {
            sent,
            tag,
            idle_reply,
            ..
        } = self;
        sent.feed(&buf[..n], |head| {
            // A command (`a12 IDLE`); DONE and literals carry no tag.
            let word = first_word(head);
            if word.len() > 1 && word[0] == b'a' && word[1..].iter().all(u8::is_ascii_digit) {
                *tag = word.to_vec();
                let verb = first_word(head.get(word.len() + 1..).unwrap_or_default());
                *idle_reply = verb.eq_ignore_ascii_case(b"IDLE");
            }
            Ok(())
        })?;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// The byte stream under an IMAP session. Production connections are always
/// TLS; the plaintext variant exists only for the fake server in tests.
pub enum Transport {
    Tls(Box<TlsStream<TcpStream>>),
    #[cfg(test)]
    Plain(TcpStream),
}

impl Read for Transport {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Transport::Tls(s) => s.read(buf),
            #[cfg(test)]
            Transport::Plain(s) => s.read(buf),
        }
    }
}

impl Write for Transport {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Transport::Tls(s) => s.write(buf),
            #[cfg(test)]
            Transport::Plain(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Transport::Tls(s) => s.flush(),
            #[cfg(test)]
            Transport::Plain(s) => s.flush(),
        }
    }
}

impl Transport {
    fn sock(&self) -> &TcpStream {
        match self {
            Transport::Tls(s) => s.get_ref(),
            #[cfg(test)]
            Transport::Plain(s) => s,
        }
    }
}

impl SetReadTimeout for Counted<Transport> {
    /// The crate calls this with the wait's timeout before an IDLE wait and
    /// with `None` after it. The timeout becomes a deadline for the whole
    /// wait (see `DoneGuard::deadline`); `None` clears it before the crate
    /// sends DONE and reads the reply, and never leaves the socket unbounded.
    fn set_read_timeout(&mut self, t: Option<Duration>) -> imap::Result<()> {
        *self.done.deadline.lock().expect("deadline lock") = t.map(|t| Instant::now() + t);
        self.inner
            .sock()
            .set_read_timeout(Some(t.unwrap_or(IO_TIMEOUT)))
            .map_err(imap::Error::Io)
    }
}

type Session = imap::Session<Counted<Transport>>;

/// Per-address connect timeout; the last address gets [`CONNECT_TIMEOUT`].
const ADDR_TIMEOUT: Duration = Duration::from_secs(4);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const IO_TIMEOUT: Duration = Duration::from_secs(90);
/// A connection unused for longer than this is checked with a NOOP before
/// it carries a command (it may have died during sleep or a network change).
const STALE_AFTER: Duration = Duration::from_secs(60);
/// How long that NOOP may take before the connection counts as dead.
const PROBE_TIMEOUT: Duration = Duration::from_secs(8);

pub struct Imap {
    session: Session,
    /// The underlying TCP socket (under TLS), for timeouts and shutdown.
    sock: TcpStream,
    /// Whether the server has a folder selected (`selected` is only our
    /// cached choice and is reset to force a fresh SELECT).
    in_folder: bool,
    /// The selected folder and its UIDVALIDITY.
    selected: Option<(String, u32)>,
    /// UIDNEXT when that folder was selected (0: unknown).
    selected_uidnext: u32,
    has_move: bool,
    has_uidplus: bool,
    /// Folders are opened with EXAMINE and every mutating call is refused.
    read_only: bool,
    bytes: Arc<AtomicU64>,
    done: Arc<DoneGuard>,
    /// IMAP commands issued on this connection (see `commands()`).
    commands: u64,
    capabilities: Vec<String>,
    /// The server refused `LIST ... RETURN (STATUS ...)` once; don't ask again
    /// on this connection.
    list_status_broken: bool,
    /// When the connection last carried a command (wall clock, so time
    /// asleep counts).
    last_used: SystemTime,
    /// The account the diagnostics log names for this connection's
    /// commands (0: none yet, e.g. while an account is being added).
    account: i64,
}

/// The STATUS data items the sync gate needs. An item the server left out
/// is 0, so a NOMODSEQ folder has `hms == 0`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FolderStatus {
    pub messages: u32,
    pub uidnext: u32,
    pub uidvalidity: u32,
    pub hms: u64,
}

/// Folder STATUS by raw server name.
pub type Statuses = HashMap<String, FolderStatus>;

const STATUS_ITEMS: &str = "(MESSAGES UIDNEXT UIDVALIDITY HIGHESTMODSEQ)";

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteFolder {
    /// Raw server name (modified UTF-7), used in commands.
    pub name: String,
    /// Decoded last path segment, shown in the sidebar.
    pub display: String,
    pub depth: i64,
    pub role: Role,
}

pub struct Selected {
    pub uidvalidity: u32,
    pub exists: u32,
    pub uid_next: u32,
}

/// What a delete did on the server.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Expunged {
    /// The message is gone (UID EXPUNGE, or MOVE).
    Removed,
    /// Without UIDPLUS the message is only flagged \Deleted: a bare EXPUNGE
    /// would also remove whatever other clients flagged in that folder.
    LeftFlagged,
}

/// The folder's UIDVALIDITY no longer matches the cache, so cached UIDs may
/// name other messages. The action is refused; a sync resets the folder.
#[derive(Debug)]
pub struct UidValidityChanged;

impl std::fmt::Display for UidValidityChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("the folder changed on the server; refreshing it")
    }
}

impl std::error::Error for UidValidityChanged {}

/// The server rejected the username or password. Retrying with the same
/// password only adds failed logins (and risks a lockout).
#[derive(Debug)]
pub struct AuthFailed(pub String);

impl std::fmt::Display for AuthFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "sign-in rejected: {}", self.0)
    }
}

impl std::error::Error for AuthFailed {}

/// The TLS handshake failed (an untrusted or mismatched certificate, or
/// the connection broke during it).
#[derive(Debug)]
pub struct TlsFailed(pub String);

impl std::fmt::Display for TlsFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "TLS handshake failed: {}", self.0)
    }
}

impl std::error::Error for TlsFailed {}

/// Whether a tagged NO to LOGIN rejects the credentials. imap-proto leaves
/// response codes it doesn't know (all of RFC 5530's) in the text. Any other
/// code (`[UNAVAILABLE]`, `[INUSE]`, `[LIMIT]`, ...) is a transient refusal.
/// A NO without a code is taken as a wrong password, unless its text says
/// it is about connections or temporary (Dovecot's "Maximum number of
/// connections from user+IP exceeded", "Temporary authentication failure").
fn rejects_credentials(text: &str) -> bool {
    let upper = text.trim_start().to_ascii_uppercase();
    if upper.starts_with('[') {
        return [
            "[AUTHENTICATIONFAILED]",
            "[AUTHORIZATIONFAILED]",
            "[EXPIRED]",
        ]
        .iter()
        .any(|code| upper.starts_with(code));
    }
    !transient_text(&upper)
}

/// Whether an upper-cased refusal text says "for now": too many
/// connections, temporary, or try again later.
pub fn transient_text(upper: &str) -> bool {
    [
        "TOO MANY",
        "CONNECTION LIMIT",
        "MAXIMUM NUMBER OF CONNECTIONS",
        "TEMPORAR",
        "TRY AGAIN",
        "UNAVAILABLE",
    ]
    .iter()
    .any(|t| upper.contains(t))
}

/// The response code a NO or BAD text starts with (`[OVERQUOTA] ...` gives
/// `OVERQUOTA`), upper-cased; only the code's atom, never the free text.
pub fn response_code(text: &str) -> Option<String> {
    let code = text.trim_start().strip_prefix('[')?;
    let end = code.find([']', ' ']).unwrap_or(code.len());
    let code = &code[..end];
    (!code.is_empty()
        && code.len() <= 32
        && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
    .then(|| code.to_ascii_uppercase())
}

/// A failed command's status for the diagnostics log: `NO`/`BAD` with its
/// response code if any, or the kind of failure. Never the server's text.
fn status_of(e: &imap::Error) -> String {
    let (status, text) = match e {
        imap::Error::No(t) => ("NO", t),
        imap::Error::Bad(t) => ("BAD", t),
        imap::Error::Io(io) if violation(io) => return "REFUSED".into(),
        imap::Error::Io(_) => return "IO".into(),
        imap::Error::ConnectionLost => return "LOST".into(),
        imap::Error::Append => return "NO".into(),
        imap::Error::Parse(_) => return "PARSE".into(),
        _ => return "ERROR".into(),
    };
    match response_code(text) {
        Some(code) => format!("{status} [{code}]"),
        None => status.into(),
    }
}

/// The server refused STARTTLS, or did not answer it with OK.
#[derive(Debug)]
pub struct StartTlsRefused;

impl std::fmt::Display for StartTlsRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("server refused STARTTLS")
    }
}

impl std::error::Error for StartTlsRefused {}

/// A connection that failed, and the step it failed in.
pub type StepFailure = (CheckStep, anyhow::Error);

impl Imap {
    pub fn connect(
        server: &Server,
        pin: Option<&[u8]>,
        username: &str,
        password: &str,
    ) -> Result<Self> {
        Self::connect_steps(server, pin, username, password, None).map_err(|(_, e)| e)
    }

    /// [`Imap::connect`] by steps: name lookup, TCP, TLS, then sign-in. With
    /// a `report`, each step that passed is reported to it, and a TLS
    /// failure on an implicit-TLS port also checks whether the port speaks
    /// plain text (the workers' connections do not).
    pub fn connect_steps(
        server: &Server,
        pin: Option<&[u8]>,
        username: &str,
        password: &str,
        mut report: Option<&mut dyn FnMut(CheckStep)>,
    ) -> Result<Self, StepFailure> {
        let probing = report.is_some();
        let mut passed = |step| {
            if let Some(report) = report.as_deref_mut() {
                report(step);
            }
        };
        let addrs: Vec<_> = (|| {
            let addrs: Vec<_> = (server.host.as_str(), server.port)
                .to_socket_addrs()
                .with_context(|| format!("resolving {}", server.host))?
                .collect();
            if addrs.is_empty() {
                anyhow::bail!("{} has no address", server.host);
            }
            Ok(addrs)
        })()
        .map_err(|e| (CheckStep::Resolve, e))?;
        passed(CheckStep::Resolve);
        let tcp = connect_any(&interleave(addrs))
            .with_context(|| format!("connecting to {}:{}", server.host, server.port))
            .map_err(|e| (CheckStep::Connect, e))?;
        passed(CheckStep::Connect);
        let secure = |tcp: TcpStream| -> Result<(Transport, TcpStream, Option<Vec<String>>)> {
            tcp.set_read_timeout(Some(IO_TIMEOUT))?;
            tcp.set_write_timeout(Some(IO_TIMEOUT))?;
            let tls = super::pin::connector(pin)?;
            let tcp = match server.security {
                Security::Tls => tcp,
                Security::StartTls => starttls(tcp).context("IMAP STARTTLS failed")?,
            };
            // A second handle to the plain socket, for timeouts and shutdown.
            let sock = tcp.try_clone()?;
            let stream = tls.connect(&server.host, tcp).map_err(|e| {
                if handshake_dropped(&e) {
                    // The connection broke, which says nothing about the
                    // certificate: an ordinary network failure.
                    anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionAborted))
                        .context(format!("TLS handshake with {} was cut off", server.host))
                } else if super::pin::usable(pin).is_some() {
                    // With a saved certificate a failed handshake means the
                    // server no longer presents it (changed, or expired).
                    anyhow::Error::new(super::pin::PinMismatch(server.host.clone()))
                } else {
                    TlsFailed(format!("{}: {e}", server.host)).into()
                }
            })?;
            // Before anything is sent: the certificate is the saved one.
            super::pin::check_peer(&stream, pin, &server.host)?;
            let mut stream = Transport::Tls(Box::new(stream));
            // Capabilities learned before TLS are attacker data: after a
            // STARTTLS they are asked for again.
            let caps = secured_greeting(
                &mut stream,
                server.security == Security::Tls,
                server.security == Security::StartTls,
            )?;
            Ok((stream, sock, caps))
        };
        let (stream, sock, caps) = secure(tcp).map_err(|e| {
            let plain = probing
                && server.security == Security::Tls
                && e.chain()
                    .any(|c| c.is::<TlsFailed>() || c.is::<super::pin::PinMismatch>())
                && answers_plain(&server.host, server.port);
            // A server that hangs up before it greets was not reached.
            let step = if e.chain().any(|c| c.is::<ConnectionClosed>()) {
                CheckStep::Connect
            } else {
                CheckStep::Secure
            };
            (step, if plain { e.context(PlainAnswer) } else { e })
        })?;
        passed(CheckStep::Secure);
        let mut imap = Self::login(stream, sock, username, password, caps.as_deref())
            .map_err(|e| (CheckStep::ImapSignIn, e))?;
        if probing {
            imap.noop().map_err(|e| (CheckStep::ImapSignIn, e))?;
        }
        passed(CheckStep::ImapSignIn);
        Ok(imap)
    }

    /// Plaintext connection to a local fake server (tests only).
    #[cfg(test)]
    pub(crate) fn connect_plain(
        addr: impl ToSocketAddrs,
        username: &str,
        password: &str,
    ) -> Result<Self> {
        Self::connect_plain_with(addr, username, password, false)
    }

    /// [`Imap::connect_plain`] that asks for the capabilities before the
    /// sign-in, as a connection does after STARTTLS (tests only).
    #[cfg(test)]
    pub(crate) fn connect_plain_listing(
        addr: impl ToSocketAddrs,
        username: &str,
        password: &str,
    ) -> Result<Self> {
        Self::connect_plain_with(addr, username, password, true)
    }

    #[cfg(test)]
    fn connect_plain_with(
        addr: impl ToSocketAddrs,
        username: &str,
        password: &str,
        list: bool,
    ) -> Result<Self> {
        let tcp = TcpStream::connect(addr)?;
        tcp.set_read_timeout(Some(IO_TIMEOUT))?;
        tcp.set_write_timeout(Some(IO_TIMEOUT))?;
        let sock = tcp.try_clone()?;
        let mut stream = Transport::Plain(tcp);
        let caps = secured_greeting(&mut stream, true, list)?;
        Self::login(stream, sock, username, password, caps.as_deref())
    }

    /// Sign in and read the capabilities. `caps` are the capabilities listed
    /// before the sign-in, if they were asked for: a server that lists
    /// `LOGINDISABLED` under TLS is refused before anything is sent.
    ///
    /// The sign-in is `LOGIN`, never `AUTHENTICATE`: the crate's
    /// `authenticate` matches the server's challenge with a regular
    /// expression, which links the whole regex engine into the program
    /// (976 KB, over the size ceiling).
    fn login(
        stream: Transport,
        sock: TcpStream,
        username: &str,
        password: &str,
        caps: Option<&[String]>,
    ) -> Result<Self> {
        let bytes = Arc::new(AtomicU64::new(0));
        let done = Arc::new(DoneGuard::default());
        let client = imap::Client::new(Counted::new(stream, bytes.clone(), done.clone()));
        let listed =
            |name: &str| caps.is_some_and(|c| c.iter().any(|c| c.eq_ignore_ascii_case(name)));
        if listed("LOGINDISABLED") {
            return Err(NoSignIn.into());
        }
        let refused = |e: imap::Error| -> anyhow::Error {
            match e {
                imap::Error::No(text) if rejects_credentials(&text) => AuthFailed(text).into(),
                // Kept typed, so an I/O failure counts as a connection error.
                e => anyhow::Error::new(e).context("sign-in failed"),
            }
        };
        let mut session = client
            .login(username, password)
            .map_err(|(e, _)| refused(e))?;
        let capabilities: Vec<String> = match session.capabilities() {
            Ok(caps) => caps
                .iter()
                .map(|c| match c {
                    Capability::Imap4rev1 => "IMAP4rev1".to_string(),
                    Capability::Auth(a) => format!("AUTH={a}"),
                    Capability::Atom(a) => a.to_string(),
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        let has = |name: &str| capabilities.iter().any(|c| c.eq_ignore_ascii_case(name));
        // Names and flags would be UTF-8 where Komorebi sends and reads
        // modified UTF-7 (UTF8=ACCEPT servers still take the latter).
        if has("UTF8=ONLY") {
            return Err(Utf8Only.into());
        }
        Ok(Self {
            has_move: has("MOVE"),
            has_uidplus: has("UIDPLUS"),
            session,
            sock,
            selected: None,
            selected_uidnext: 0,
            in_folder: false,
            read_only: false,
            bytes,
            done,
            commands: 2, // sign-in + CAPABILITY
            capabilities,
            list_status_broken: false,
            last_used: SystemTime::now(),
            account: 0,
        })
    }

    /// Name `account` in the diagnostics log lines of this connection.
    pub fn set_account(&mut self, account: i64) {
        self.account = account;
    }

    /// Run one command on the session: every command goes through here,
    /// so it is counted, and timed for the diagnostics log under `verb`
    /// (with its status or response code only, never its arguments).
    fn run<T>(
        &mut self,
        verb: &'static str,
        f: impl FnOnce(&mut Session) -> imap::Result<T>,
    ) -> imap::Result<T> {
        self.commands += 1;
        let started = Instant::now();
        let result = f(&mut self.session);
        if diag::enabled() {
            let status = match &result {
                Ok(_) => "OK".to_string(),
                Err(e) => status_of(e),
            };
            diag::record(self.account, verb, &status, started.elapsed());
        }
        result
    }

    /// Open folders with EXAMINE and refuse every call that could change the
    /// mailbox (flags, moves, deletes, appends).
    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
        self.selected = None;
    }

    fn writable(&self) -> Result<()> {
        if self.read_only {
            anyhow::bail!("this IMAP connection is read-only");
        }
        Ok(())
    }

    /// The underlying TCP socket, e.g. to set timeouts or shut it down from
    /// another thread.
    pub fn socket(&self) -> &TcpStream {
        &self.sock
    }

    /// IMAP payload bytes sent and received on this connection so far.
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    /// IMAP commands issued on this connection so far, LOGIN included.
    pub fn commands(&self) -> u64 {
        self.commands
    }

    /// Capabilities advertised after login, e.g. `IDLE`, `AUTH=PLAIN`.
    pub fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    pub fn has_capability(&self, name: &str) -> bool {
        self.capabilities
            .iter()
            .any(|c| c.eq_ignore_ascii_case(name))
    }

    /// Run a UID FETCH through the normal parser and count the results
    /// (checks whether the parser understands a server extension).
    pub fn fetch_count(&mut self, uid_set: &str, query: &str) -> Result<usize> {
        Ok(self.run("FETCH", |s| s.uid_fetch(uid_set, query))?.len())
    }

    /// Send a raw command and return the untagged response bytes. A
    /// read-only connection allows only commands that change nothing.
    pub fn raw(&mut self, command: &str) -> Result<Vec<u8>> {
        if self.read_only && !reads_only(command) {
            anyhow::bail!("this IMAP connection is read-only");
        }
        Ok(self.run("RAW", |s| s.run_command_and_read_response(command))?)
    }

    pub fn noop(&mut self) -> Result<()> {
        Ok(self.run("NOOP", |s| s.noop())?)
    }

    /// IDLE on the selected folder (RFC 2177) until the server reports a
    /// change or `timeout` passes, then end it with DONE. A tagged NO or BAD
    /// comes back as an error, after which the connection must be dropped
    /// (not logged out). Any other first reply than the `+` continuation
    /// (a tagged OK would reach the crate's `unreachable!()`) is refused by
    /// the stream ([`Counted`]) and fails the connection the same way.
    pub fn idle(&mut self, timeout: Duration) -> Result<WaitOutcome> {
        let done = self.done.clone();
        done.refuse.store(true, Ordering::Relaxed);
        let outcome = self.run("IDLE", |s| {
            let handle = s.idle()?;
            done.refuse.store(false, Ordering::Relaxed);
            handle.wait_with_timeout(timeout)
        });
        self.sock.set_read_timeout(Some(IO_TIMEOUT))?;
        let outcome = match outcome? {
            imap::extensions::idle::WaitOutcome::TimedOut => WaitOutcome::TimedOut,
            imap::extensions::idle::WaitOutcome::MailboxChanged => WaitOutcome::MailboxChanged,
        };
        if self.done.failed.load(Ordering::Relaxed) {
            // DONE's reply wasn't read: the next tagged reply would not match.
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "IDLE did not end cleanly",
            )
            .into());
        }
        Ok(outcome)
    }

    /// Note that the connection just carried a command.
    pub fn touch(&mut self) {
        self.last_used = SystemTime::now();
    }

    /// If the connection sat unused for over a minute, check it with a
    /// short NOOP; an error means it is dead and should be replaced.
    pub fn check_if_stale(&mut self) -> Result<()> {
        // A clock that went back counts as stale too.
        if self
            .last_used
            .elapsed()
            .is_ok_and(|idle| idle <= STALE_AFTER)
        {
            return Ok(());
        }
        self.sock.set_read_timeout(Some(PROBE_TIMEOUT))?;
        // An UNSELECT that went through proves the connection alive, so the
        // NOOP is only needed when no folder was open.
        let was_in_folder = self.in_folder && self.has_capability("UNSELECT");
        let result = self
            .leave_folder()
            .and_then(|()| if was_in_folder { Ok(()) } else { self.noop() });
        self.sock.set_read_timeout(Some(IO_TIMEOUT))?;
        self.drain_unsolicited();
        result
    }

    #[cfg(test)]
    pub(crate) fn set_last_used(&mut self, at: SystemTime) {
        self.last_used = at;
    }

    pub fn list_folders(&mut self) -> Result<Vec<RemoteFolder>> {
        let names = self.run("LIST", |s| s.list(Some(""), Some("*")))?;
        Ok(to_remote(&names))
    }

    /// The folder list and every folder's STATUS in one round trip
    /// (LIST-STATUS, RFC 5819). `None` if the server can't do that, or once
    /// refused it on this connection. Asking for HIGHESTMODSEQ needs no
    /// ENABLE: it turns CONDSTORE on implicitly (RFC 7162 §3.1).
    pub fn list_status(&mut self) -> Result<Option<(Vec<RemoteFolder>, Statuses)>> {
        if self.list_status_broken
            || !["LIST-STATUS", "LIST-EXTENDED", "CONDSTORE"]
                .iter()
                .all(|c| self.has_capability(c))
        {
            return Ok(None);
        }
        self.leave_folder()?;
        self.drain_unsolicited();
        // The crate puts the pattern into the command as-is.
        let pattern = format!("\"*\" RETURN (STATUS {STATUS_ITEMS})");
        let names = match self.run("LIST-STATUS", |s| s.list(Some(""), Some(&pattern))) {
            Ok(names) => names,
            // A tagged NO/BAD leaves the stream in step.
            Err(imap::Error::No(_) | imap::Error::Bad(_)) => {
                self.list_status_broken = true;
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        };
        let folders = to_remote(&names);
        Ok(Some((folders, self.take_statuses())))
    }

    /// One folder's STATUS, or `None` if the server refused it (e.g. the
    /// folder is gone) or sent no STATUS line. Only with CONDSTORE, since
    /// the sync gate is useless without HIGHESTMODSEQ.
    pub fn status(&mut self, folder: &str) -> Result<Option<FolderStatus>> {
        if !self.has_capability("CONDSTORE") {
            return Ok(None);
        }
        // Also keeps STATUS off the selected folder (RFC 3501 §6.3.10).
        self.leave_folder()?;
        self.drain_unsolicited();
        match self.run("STATUS", |s| s.status(folder, STATUS_ITEMS)) {
            Ok(_) => {}
            Err(imap::Error::No(_) | imap::Error::Bad(_)) => return Ok(None),
            Err(e) => return Err(e.into()),
        }
        Ok(self.take_statuses().remove(folder))
    }

    /// The STATUS lines queued by the last command, by folder name.
    fn take_statuses(&mut self) -> Statuses {
        fold_statuses(self.session.unsolicited_responses.try_iter())
    }

    /// Leave the selected folder with UNSELECT (RFC 3691), which, unlike
    /// CLOSE, never expunges. While a folder is selected the server may send
    /// legal unsolicited responses (`* n FETCH (FLAGS ...)`, `* OK [...]`)
    /// that the crate's NOOP, SEARCH and LIST parsers reject, failing the
    /// command and desynchronizing the stream; with nothing selected none
    /// arrive. Without UNSELECT this does nothing.
    pub fn leave_folder(&mut self) -> Result<()> {
        if !self.in_folder || !self.has_capability("UNSELECT") {
            return Ok(());
        }
        self.run("UNSELECT", |s| s.run_command_and_check_ok("UNSELECT"))?;
        self.in_folder = false;
        self.selected = None;
        Ok(())
    }

    /// Discard queued unsolicited responses. Nothing acts on them, and the
    /// crate's channel would otherwise grow for the connection's lifetime.
    pub fn drain_unsolicited(&mut self) {
        while self.session.unsolicited_responses.try_recv().is_ok() {}
    }

    pub fn select(&mut self, folder: &str) -> Result<Selected> {
        // A failed SELECT leaves no folder selected (RFC 3501 §6.3.1).
        self.selected = None;
        self.in_folder = false;
        let mb = if self.read_only {
            self.run("EXAMINE", |s| s.examine(folder))
        } else {
            self.run("SELECT", |s| s.select(folder))
        }
        .with_context(|| format!("opening folder {folder}"))?;
        let uidvalidity = mb.uid_validity.unwrap_or(0);
        self.selected = Some((folder.to_string(), uidvalidity));
        self.selected_uidnext = mb.uid_next.unwrap_or(0);
        self.in_folder = true;
        Ok(Selected {
            uidvalidity,
            exists: mb.exists,
            uid_next: mb.uid_next.unwrap_or(0),
        })
    }

    /// Select `folder` unless it already is, and check that its UIDVALIDITY
    /// is still `expected` (the cached value; 0 skips the check). A mismatch
    /// on an earlier selection is rechecked with a fresh SELECT.
    /// Make `folder` the selected folder, re-SELECTing if it isn't, or if
    /// `max_uid` is at or above the UIDNEXT it had when selected: some servers
    /// (seen on Postale's Dovecot) never show a long-open selection mail that
    /// arrived later, not even after NOOP, and a UID STORE or MOVE on an
    /// unknown UID succeeds without doing anything.
    fn ensure_selected(&mut self, folder: &str, expected: u32, max_uid: u32) -> Result<()> {
        let fresh_enough = self.selected_uidnext == 0 || max_uid < self.selected_uidnext;
        let uidvalidity = match &self.selected {
            Some((name, v))
                if name == folder && (expected == 0 || *v == expected) && fresh_enough =>
            {
                *v
            }
            _ => self.select(folder)?.uidvalidity,
        };
        if expected != 0 && uidvalidity != expected {
            return Err(UidValidityChanged.into());
        }
        Ok(())
    }

    /// UID → flag bits of every message in the selected folder, in one
    /// round trip (replaces SEARCH ALL/UNSEEN/FLAGGED/ANSWERED).
    pub fn uid_flags(&mut self) -> Result<HashMap<u32, u8>> {
        self.flags_of("1:*", "(UID FLAGS)")
    }

    /// UID → flag bits of the selected folder's messages whose flags changed
    /// since mod-sequence `modseq` (CHANGEDSINCE, RFC 7162 §3.1.4). Expunged
    /// messages are not reported.
    pub fn uid_flags_changed(&mut self, modseq: u64) -> Result<HashMap<u32, u8>> {
        self.flags_of("1:*", &format!("(UID FLAGS) (CHANGEDSINCE {modseq})"))
    }

    /// UID → flag bits of `uids` in the selected folder.
    pub fn uid_flags_of(&mut self, uids: &[u32]) -> Result<HashMap<u32, u8>> {
        if uids.is_empty() {
            return Ok(HashMap::new());
        }
        self.flags_of(&uid_set(uids), "(UID FLAGS)")
    }

    fn flags_of(&mut self, set: &str, query: &str) -> Result<HashMap<u32, u8>> {
        let fetches = self.run("FETCH", |s| s.uid_fetch(set, query))?;
        Ok(fetches
            .iter()
            .filter_map(|f| Some((f.uid?, flag_bits(f.flags()))))
            .collect())
    }

    /// Every UID in the selected folder (UID SEARCH ALL; ESEARCH replies
    /// can't be parsed, so none is asked for).
    pub fn uids(&mut self) -> Result<HashSet<u32>> {
        self.search("ALL")
    }

    /// UIDs in the selected folder matching a SEARCH `query`.
    pub fn search(&mut self, query: &str) -> Result<HashSet<u32>> {
        Ok(self.run("SEARCH", |s| s.uid_search(query))?)
    }

    /// The server's INTERNALDATE (delivery time, Unix seconds) per UID.
    pub fn internal_dates(&mut self, uids: &[u32]) -> Result<Vec<(u32, i64)>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let fetches = self.run("FETCH", |s| {
            s.uid_fetch(uid_set(uids), "(UID INTERNALDATE)")
        })?;
        Ok(fetches
            .iter()
            .filter_map(|f| Some((f.uid?, f.internal_date()?.timestamp())))
            .collect())
    }

    pub fn fetch_headers(&mut self, uids: &[u32]) -> Result<Vec<NewHeader>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        self.headers_of(uid_set(uids))
    }

    /// Headers of the messages with UID `first` and above. `first:*` always
    /// includes the highest UID (RFC 3501 §6.4.8), so a lower one is dropped.
    pub fn fetch_headers_from(&mut self, first: u32) -> Result<Vec<NewHeader>> {
        let mut headers = self.headers_of(format!("{first}:*"))?;
        headers.retain(|h| h.uid >= first);
        Ok(headers)
    }

    fn headers_of(&mut self, set: String) -> Result<Vec<NewHeader>> {
        let query = format!(
            "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER.FIELDS ({HEADER_FIELDS})])"
        );
        let fetches = self.run("FETCH", |s| s.uid_fetch(set, query))?;
        Ok(fetches
            .iter()
            .filter_map(|f| {
                let uid = f.uid?;
                let bits = flag_bits(f.flags());
                // Flagged \Deleted since the flags fetch: hidden like the rest.
                if bits & db::DELETED != 0 {
                    return None;
                }
                let mut h = parse_header(f.header().unwrap_or_default());
                h.uid = uid;
                h.size = f.size.unwrap_or(0) as i64;
                let internal = f.internal_date().map(|d| d.timestamp());
                h.date = message_date(h.date, internal);
                h.seen = bits & db::SEEN != 0;
                h.flagged = bits & db::FLAGGED != 0;
                h.answered = bits & db::ANSWERED != 0;
                Some(h)
            })
            .collect())
    }

    /// Full RFC 822 bytes of messages, without setting \Seen.
    /// `uidvalidity` is the folder's cached value (see `ensure_selected`).
    pub fn fetch_bodies(
        &mut self,
        (folder, uidvalidity): (&str, u32),
        uids: &[u32],
    ) -> Result<Vec<(u32, Vec<u8>)>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let max_uid = uids.iter().copied().max().unwrap_or(0);
        self.ensure_selected(folder, uidvalidity, max_uid)?;
        let fetches = self.run("FETCH", |s| s.uid_fetch(uid_set(uids), "(UID BODY.PEEK[])"))?;
        Ok(fetches
            .iter()
            .filter_map(|f| Some((f.uid?, f.body()?.to_vec())))
            .collect())
    }

    /// Set or clear `flag` on `uids` in `folder`: one STORE per UID set
    /// (see [`uid_sets`]).
    pub fn set_flags(
        &mut self,
        (folder, uidvalidity): (&str, u32),
        uids: &[u32],
        flag: &str,
        on: bool,
    ) -> Result<()> {
        self.writable()?;
        let Some(&max_uid) = uids.iter().max() else {
            return Ok(());
        };
        self.ensure_selected(folder, uidvalidity, max_uid)?;
        let op = if on { "+FLAGS.SILENT" } else { "-FLAGS.SILENT" };
        for set in uid_sets(uids, MAX_UID_SET) {
            self.run("STORE", |s| s.uid_store(set, format!("{op} ({flag})")))?;
        }
        Ok(())
    }

    /// Mark every message of `folder` up to UID `through` read: one STORE
    /// on `1:through`, so mail that arrived after the last sync stays unread.
    pub fn mark_read_through(
        &mut self,
        (folder, uidvalidity): (&str, u32),
        through: u32,
    ) -> Result<()> {
        self.writable()?;
        if through == 0 {
            return Ok(());
        }
        self.ensure_selected(folder, uidvalidity, through)?;
        self.run("STORE", |s| {
            s.uid_store(format!("1:{through}"), "+FLAGS.SILENT (\\Seen)")
        })?;
        Ok(())
    }

    /// Move messages to another folder (MOVE, or COPY + delete), a UID set
    /// at a time, so a failure leaves each set either moved or untouched.
    pub fn move_messages(
        &mut self,
        (folder, uidvalidity): (&str, u32),
        uids: &[u32],
        dest: &str,
    ) -> Result<Expunged> {
        self.writable()?;
        // The crate sends COPY's mailbox name unquoted; a name it can't
        // quote is refused before any I/O.
        let quoted = if self.has_move {
            None
        } else {
            Some(quote(dest)?)
        };
        let Some(&max_uid) = uids.iter().max() else {
            return Ok(Expunged::Removed);
        };
        self.ensure_selected(folder, uidvalidity, max_uid)?;
        let mut done = Expunged::Removed;
        for set in uid_sets(uids, MAX_UID_SET) {
            match &quoted {
                None => self.run("MOVE", |s| s.uid_mv(&set, dest))?,
                // A failed COPY returns here, before the originals are flagged.
                Some(dest) => {
                    self.run("COPY", |s| {
                        s.run_command_and_check_ok(format!("UID COPY {set} {dest}"))
                    })?;
                    done = self.expunge(&set)?;
                }
            }
        }
        Ok(done)
    }

    /// Remove messages from the selected folder (see [`Expunged`]).
    pub fn delete_messages(
        &mut self,
        (folder, uidvalidity): (&str, u32),
        uids: &[u32],
    ) -> Result<Expunged> {
        self.writable()?;
        let Some(&max_uid) = uids.iter().max() else {
            return Ok(Expunged::Removed);
        };
        self.ensure_selected(folder, uidvalidity, max_uid)?;
        let mut done = Expunged::Removed;
        for set in uid_sets(uids, MAX_UID_SET) {
            done = self.expunge(&set)?;
        }
        Ok(done)
    }

    fn expunge(&mut self, set: &str) -> Result<Expunged> {
        self.run("STORE", |s| s.uid_store(set, r"+FLAGS.SILENT (\Deleted)"))?;
        if !self.has_uidplus {
            // A bare EXPUNGE would also remove anything another client
            // flagged \Deleted here, so the messages stay flagged (and hidden).
            return Ok(Expunged::LeftFlagged);
        }
        self.run("EXPUNGE", |s| s.uid_expunge(set))?;
        Ok(Expunged::Removed)
    }

    /// Save a copy of a sent message to `folder`, flagged \Seen.
    pub fn append(&mut self, folder: &str, raw: &[u8]) -> Result<()> {
        self.append_flagged(folder, raw, Flag::Seen)
    }

    /// Save a draft to `folder`, flagged \Draft only. The crate drops the
    /// tagged reply, so an APPENDUID in it can't be read: find the copy
    /// with [`Imap::find_message_id`].
    pub fn append_draft(&mut self, folder: &str, raw: &[u8]) -> Result<()> {
        self.append_flagged(folder, raw, Flag::Draft)
    }

    /// The (UIDVALIDITY, UID) of the newest message in `folder` that is not
    /// flagged \Deleted and has Message-ID `message_id`. The folder is
    /// selected afresh, so it shows a message appended just now.
    pub fn find_message_id(
        &mut self,
        folder: &str,
        message_id: &str,
    ) -> Result<Option<(u32, u32)>> {
        let uidvalidity = self.select(folder)?.uidvalidity;
        let query = format!("UNDELETED HEADER Message-ID {}", quote(message_id)?);
        Ok(self
            .search(&query)?
            .into_iter()
            .max()
            .filter(|_| uidvalidity != 0)
            .map(|uid| (uidvalidity, uid)))
    }

    fn append_flagged(&mut self, folder: &str, raw: &[u8], flag: Flag<'static>) -> Result<()> {
        self.writable()?;
        // The crate puts the name between quotes without escaping it.
        if folder.contains(['"', '\\', '\r', '\n', '\0']) {
            anyhow::bail!(
                "can't save to {folder:?}: the name has a quote, backslash or line break"
            );
        }
        self.run("APPEND", |s| s.append_with_flags(folder, raw, &[flag]))
            .with_context(|| format!("saving a copy to {folder}"))
    }

    pub fn logout(mut self) {
        self.run("LOGOUT", |s| s.logout()).ok();
    }
}

/// Fold STATUS responses into one [`FolderStatus`] per folder name. EXISTS,
/// EXPUNGE and RECENT for the selected folder are dropped: a sync reads its
/// own snapshot after SELECT.
fn fold_statuses(responses: impl Iterator<Item = UnsolicitedResponse>) -> Statuses {
    let mut out = Statuses::new();
    for r in responses {
        let UnsolicitedResponse::Status {
            mailbox,
            attributes,
        } = r
        else {
            continue;
        };
        let st = out.entry(mailbox).or_default();
        for a in attributes {
            match a {
                StatusAttribute::Messages(n) => st.messages = n,
                StatusAttribute::UidNext(n) => st.uidnext = n,
                StatusAttribute::UidValidity(n) => st.uidvalidity = n,
                StatusAttribute::HighestModSeq(n) => st.hms = n,
                _ => {}
            }
        }
    }
    out
}

/// Selectable folders of a LIST reply.
fn to_remote(names: &[Name]) -> Vec<RemoteFolder> {
    let mut out = Vec::new();
    for n in names {
        let attrs = n.attributes();
        // A name that can't be selected is a header (`\Noselect`) or not
        // there at all (`\NonExistent`): neither is a folder to sync.
        let unselectable = |a: &NameAttribute| match a {
            NameAttribute::NoSelect => true,
            NameAttribute::Custom(v) => v.eq_ignore_ascii_case("\\NonExistent"),
            _ => false,
        };
        if attrs.iter().any(unselectable) {
            continue;
        }
        let special = attrs.iter().find_map(|a| match a {
            NameAttribute::Custom(v) => Role::from_attr(v),
            _ => None,
        });
        out.push(remote_folder(n.name(), n.delimiter(), special));
    }
    out
}

pub fn flag_bits(flags: &[Flag]) -> u8 {
    flags.iter().fold(0, |bits, f| {
        bits | match f {
            Flag::Seen => db::SEEN,
            Flag::Flagged => db::FLAGGED,
            Flag::Answered => db::ANSWERED,
            Flag::Deleted => db::DELETED,
            _ => 0,
        }
    })
}

/// A header's Date, unless it is missing or more than a day ahead of the
/// server's INTERNALDATE (bogus or spam dates), in which case the latter.
fn message_date(header: i64, internal: Option<i64>) -> i64 {
    match internal {
        Some(internal) if header == 0 || header > internal + 86_400 => internal,
        _ => header,
    }
}

/// Whether a raw command only reads (FETCH, SEARCH, STATUS, LIST, NOOP or
/// CAPABILITY, with or without UID) and is a single line.
fn reads_only(command: &str) -> bool {
    let mut words = command.split(' ');
    let mut verb = words.next().unwrap_or_default();
    if verb.eq_ignore_ascii_case("UID") {
        verb = words.next().unwrap_or_default();
    }
    !command.contains(['\r', '\n', '\0'])
        && ["FETCH", "SEARCH", "STATUS", "LIST", "NOOP", "CAPABILITY"]
            .iter()
            .any(|v| v.eq_ignore_ascii_case(verb))
}

/// An IMAP quoted string (RFC 3501 §4.3).
fn quote(s: &str) -> Result<String> {
    if s.contains(['\r', '\n', '\0']) {
        anyhow::bail!("folder name {s:?} contains a line break or NUL");
    }
    Ok(format!(
        "\"{}\"",
        s.replace('\\', r"\\").replace('"', "\\\"")
    ))
}

/// Resolved addresses in connect order: alternating families, starting
/// with the first one's (RFC 8305 §4).
pub(crate) fn interleave(addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let first_v6 = addrs.first().is_some_and(SocketAddr::is_ipv6);
    let (first, other): (Vec<_>, Vec<_>) = addrs.into_iter().partition(|a| a.is_ipv6() == first_v6);
    let (mut first, mut other) = (first.into_iter(), other.into_iter());
    let mut out = Vec::new();
    loop {
        match (first.next(), other.next()) {
            (None, None) => return out,
            (a, b) => out.extend(a.into_iter().chain(b)),
        }
    }
}

/// Connect to the first address that answers, a few seconds each, so one
/// dead address (e.g. IPv6 without a route) doesn't cost the full timeout.
pub(crate) fn connect_any(addrs: &[SocketAddr]) -> Result<TcpStream> {
    let mut last_err = None;
    for (i, addr) in addrs.iter().enumerate() {
        let timeout = if i + 1 == addrs.len() {
            CONNECT_TIMEOUT
        } else {
            ADDR_TIMEOUT
        };
        match TcpStream::connect_timeout(addr, timeout) {
            Ok(tcp) => {
                tcp.set_nodelay(true)?;
                return Ok(tcp);
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.map_or_else(|| anyhow!("no address"), Into::into))
}

/// A TLS port that answered in plain text (the failed handshake's cause).
#[derive(Debug)]
pub struct PlainAnswer;

impl std::fmt::Display for PlainAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("the port answered without encryption")
    }
}

impl std::error::Error for PlainAnswer {}

/// Whether a server on `host:port` speaks first in plain text (an IMAP
/// `* OK` or `+OK`, an SMTP `220`), which a TLS port never does. Read-only:
/// connects, waits a moment for a greeting, reads at most a few bytes and
/// sends nothing.
pub(crate) fn answers_plain(host: &str, port: u16) -> bool {
    let Ok(addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    let Ok(mut tcp) = connect_any(&interleave(addrs.collect())) else {
        return false;
    };
    let mut buf = [0u8; 8];
    tcp.set_read_timeout(Some(Duration::from_secs(2))).ok();
    let n = tcp.read(&mut buf).unwrap_or(0);
    let head = &buf[..n];
    head.starts_with(b"* OK") || head.starts_with(b"+OK") || head.starts_with(b"220")
}

/// Plain-text greeting and STARTTLS exchange, before the TLS handshake.
fn starttls<S: Read + Write>(stream: S) -> Result<S> {
    let mut reader = BufReader::new(stream);
    starttls_exchange(&mut reader)?;
    // Bytes sent before the handshake would be taken as if they had come
    // over TLS (response injection).
    if !reader.buffer().is_empty() {
        anyhow::bail!("unexpected data before TLS");
    }
    Ok(reader.into_inner())
}

/// The plain exchange that ends where the TLS handshake starts: the
/// greeting, a CAPABILITY that must list STARTTLS (a server that doesn't
/// is never asked, and never sees a password), then STARTTLS, which must be
/// answered OK. Also the Review certificate probe's.
pub(crate) fn starttls_exchange<S: Read + Write>(reader: &mut BufReader<S>) -> Result<()> {
    // Whatever it lists is attacker data until TLS: asked for again.
    greeting(reader)?;
    let caps = capability_exchange(reader, "a0")?;
    if !caps.iter().any(|c| c == "STARTTLS") {
        return Err(StartTlsRefused.into());
    }
    reader.get_mut().write_all(b"a1 STARTTLS\r\n")?;
    for _ in 0..MAX_REPLY_LINES {
        let line = super::pin::read_line_capped(reader)?;
        if let Some(rest) = line.strip_prefix("a1 ") {
            if rest.starts_with("OK") {
                return Ok(());
            }
            return Err(StartTlsRefused.into());
        }
    }
    anyhow::bail!("no answer to STARTTLS")
}

/// The most lines of one reply read by hand.
const MAX_REPLY_LINES: usize = 64;

/// The server's greeting, refused unless it is a plain `* OK`: `* PREAUTH`
/// would mean signed in without a password and `* BYE` that the server is
/// not taking this connection.
///
/// A `* OK [CAPABILITY ...]` greeting (Dovecot's) brings the capabilities
/// for free; they are returned, and only trusted under TLS.
fn greeting<R: BufRead>(reader: &mut R) -> Result<Option<Vec<String>>> {
    let line = super::pin::read_line_capped(reader)?;
    let upper = line.to_ascii_uppercase();
    Err(if let Some(rest) = upper.strip_prefix("* OK") {
        let listed = rest
            .trim_start()
            .strip_prefix("[CAPABILITY ")
            .and_then(|r| r.split_once(']'))
            .map(|(list, _)| list.split(' ').map(str::to_string).collect());
        return Ok(listed);
    } else if upper.starts_with("* PREAUTH") {
        GreetingRefused::PreAuth
    } else if upper.starts_with("* BYE") {
        GreetingRefused::Bye
    } else {
        GreetingRefused::Other
    }
    .into())
}

/// `CAPABILITY` by hand under `tag`: the upper-cased capability atoms.
fn capability_exchange<S: Read + Write>(
    reader: &mut BufReader<S>,
    tag: &str,
) -> Result<Vec<String>> {
    reader
        .get_mut()
        .write_all(format!("{tag} CAPABILITY\r\n").as_bytes())?;
    let mut caps = Vec::new();
    let prefix = format!("{tag} ");
    for _ in 0..MAX_REPLY_LINES {
        let line = super::pin::read_line_capped(reader)?;
        if let Some(rest) = line.strip_prefix(&prefix) {
            if rest.starts_with("OK") {
                return Ok(caps);
            }
            anyhow::bail!("CAPABILITY was refused");
        }
        if let Some(list) = line.trim_end().strip_prefix("* CAPABILITY ") {
            caps = list.split(' ').map(str::to_ascii_uppercase).collect();
        }
    }
    anyhow::bail!("no answer to CAPABILITY")
}

/// On the secured stream: read the greeting (unless STARTTLS did already:
/// `greet`), and, if `list`, ask for the capabilities again. Nothing the
/// server pipelined beyond the answers is accepted.
fn secured_greeting(
    stream: &mut Transport,
    greet: bool,
    list: bool,
) -> Result<Option<Vec<String>>> {
    let mut reader = BufReader::new(stream);
    let listed = if greet { greeting(&mut reader)? } else { None };
    let caps = if list {
        Some(capability_exchange(&mut reader, "c0")?)
    } else {
        listed
    };
    if !reader.buffer().is_empty() {
        anyhow::bail!("unexpected data after the greeting");
    }
    Ok(caps)
}

/// A TLS handshake that failed because the connection broke, not because of
/// what the server presented.
fn handshake_dropped(e: &native_tls::HandshakeError<TcpStream>) -> bool {
    use std::io::ErrorKind::*;
    let native_tls::HandshakeError::Failure(err) = e else {
        return false;
    };
    let mut next: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(cause) = next {
        if let Some(io) = cause.downcast_ref::<std::io::Error>()
            && matches!(
                io.kind(),
                ConnectionReset | ConnectionAborted | UnexpectedEof | TimedOut | BrokenPipe
            )
        {
            return true;
        }
        next = cause.source();
    }
    // native-tls on Windows wraps the I/O error without exposing it as a
    // source; its Debug form names the kind (`Os { code: 10053, kind:
    // ConnectionAborted, .. }`), in English whatever the language of
    // Windows.
    let debug = format!("{err:?}");
    [
        "ConnectionReset",
        "ConnectionAborted",
        "UnexpectedEof",
        "TimedOut",
        "BrokenPipe",
    ]
    .iter()
    .any(|kind| debug.contains(&format!("kind: {kind}")))
}

/// The server closed the connection (or stopped) before it answered.
#[derive(Debug)]
pub struct ConnectionClosed;

impl std::fmt::Display for ConnectionClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("connection closed")
    }
}

impl std::error::Error for ConnectionClosed {}

/// A greeting other than `* OK`: the server signed the connection in by
/// itself (PREAUTH), or said goodbye (BYE), or is not an IMAP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GreetingRefused {
    PreAuth,
    Bye,
    Other,
}

impl std::fmt::Display for GreetingRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            Self::PreAuth => "the server signed the connection in by itself (PREAUTH)",
            Self::Bye => "the server said goodbye instead of greeting (BYE)",
            Self::Other => "the server's greeting is not an IMAP greeting",
        })
    }
}

impl std::error::Error for GreetingRefused {}

/// The server speaks UTF-8 only (`UTF8=ONLY`, RFC 6855), which Komorebi
/// does not.
#[derive(Debug)]
pub struct Utf8Only;

impl std::fmt::Display for Utf8Only {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("the server only works in UTF-8 mode (UTF8=ONLY)")
    }
}

impl std::error::Error for Utf8Only {}

/// The server does not allow `LOGIN` over the secure connection
/// (`LOGINDISABLED`), the only sign-in Komorebi uses.
#[derive(Debug)]
pub struct NoSignIn;

impl std::fmt::Display for NoSignIn {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("the server does not allow LOGIN (LOGINDISABLED)")
    }
}

impl std::error::Error for NoSignIn {}

/// Longest UID set in one STORE, MOVE, COPY or EXPUNGE, so a large
/// selection stays well inside the 8,192-octet command line servers are
/// asked to accept (RFC 7162 §4).
const MAX_UID_SET: usize = 4000;

/// `uids` as one compressed UID set (`1:3,5,9:10`).
fn uid_set(uids: &[u32]) -> String {
    uid_sets(uids, usize::MAX).pop().unwrap_or_default()
}

/// `uids` sorted, without repeats, as compressed UID sets of at most
/// `max_len` bytes each (a single range may be longer).
fn uid_sets(uids: &[u32], max_len: usize) -> Vec<String> {
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut sets = Vec::new();
    let mut out = String::new();
    let mut i = 0;
    while i < sorted.len() {
        let start = sorted[i];
        let mut end = start;
        while i + 1 < sorted.len() && Some(sorted[i + 1]) == end.checked_add(1) {
            i += 1;
            end = sorted[i];
        }
        let piece = if start == end {
            start.to_string()
        } else {
            format!("{start}:{end}")
        };
        if !out.is_empty() && out.len() + 1 + piece.len() > max_len {
            sets.push(std::mem::take(&mut out));
        }
        if !out.is_empty() {
            out.push(',');
        }
        out.push_str(&piece);
        i += 1;
    }
    if !out.is_empty() {
        sets.push(out);
    }
    sets
}

fn remote_folder(name: &str, delimiter: Option<&str>, special: Option<Role>) -> RemoteFolder {
    let (leaf, depth) = match delimiter.filter(|d| !d.is_empty()) {
        Some(d) => (
            name.rsplit(d).next().unwrap_or(name),
            name.matches(d).count() as i64,
        ),
        None => (name, 0),
    };
    // Match well-known names on the decoded name; the top-level INBOX always
    // wins over special-use attributes.
    let display = decode_mutf7(leaf);
    // Only a folder at the top, or directly under INBOX (`INBOX.Sent`, the
    // way Courier and Cyrus name them), is a Sent folder by its name: a
    // `Projects/Archive` is the user's own.
    let parent = delimiter
        .filter(|d| !d.is_empty())
        .and_then(|d| name.rsplit_once(d))
        .map_or("", |(parent, _)| parent);
    let by_name = if parent.is_empty() || parent.eq_ignore_ascii_case("INBOX") {
        Role::from_name(name, &display)
    } else {
        None
    };
    let role = match by_name {
        Some(Role::Inbox) => Role::Inbox,
        by_name => special.or(by_name).unwrap_or_default(),
    };
    RemoteFolder {
        name: name.to_string(),
        display: if role == Role::Inbox {
            "Inbox".to_string()
        } else {
            capped(display)
        },
        depth: if role == Role::Other { depth } else { 0 },
        role,
    }
}

/// The longest folder name shown; commands always use the whole raw name.
const MAX_DISPLAY: usize = 120;

fn capped(display: String) -> String {
    if display.chars().count() <= MAX_DISPLAY {
        return display;
    }
    let mut short: String = display.chars().take(MAX_DISPLAY - 1).collect();
    short.push('\u{2026}');
    short
}

/// Decode an IMAP modified UTF-7 mailbox name (RFC 3501 §5.1.3).
pub fn decode_mutf7(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('-') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let encoded = &after[..end];
        if encoded.is_empty() {
            out.push('&');
        } else if let Some(text) =
            decode_mbase64(encoded).and_then(|units| String::from_utf16(&units).ok())
        {
            out.push_str(&text);
        } else {
            out.push_str(&rest[start..start + end + 2]);
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn decode_mbase64(s: &str) -> Option<Vec<u16>> {
    let mut bits = 0u32;
    let mut nbits = 0;
    let mut bytes = Vec::new();
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b',' => 63,
            _ => return None,
        } as u32;
        bits = (bits << 6) | v;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            bytes.push((bits >> nbits) as u8);
            bits &= (1 << nbits) - 1;
        }
    }
    // Whole UTF-16 units and only zero padding bits left: anything else is
    // not modified UTF-7, and the name is kept as the server wrote it.
    if bytes.len() % 2 != 0 || bits != 0 {
        return None;
    }
    Some(
        bytes
            .chunks_exact(2)
            .map(|p| u16::from_be_bytes([p[0], p[1]]))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::super::fake::*;
    use super::*;

    #[test]
    fn read_only_round_trip_against_fake_server() {
        let server = FakeServer::start(vec![
            Step::Send("* OK fake IMAP ready\r\n"),
            Step::Expect(r#"a1 LOGIN "user" "pass""#),
            Step::Send("{tag} OK LOGIN completed\r\n"),
            Step::Expect("a2 CAPABILITY"),
            Step::Send("* CAPABILITY IMAP4rev1 MOVE IDLE\r\n{tag} OK done\r\n"),
            Step::Expect("a3 LIST"),
            Step::Send(concat!(
                r#"* LIST (\HasNoChildren) "/" "INBOX""#,
                "\r\n",
                r#"* LIST (\HasNoChildren \Sent) "/" "Gesendet""#,
                "\r\n",
                r#"* LIST (\Noselect) "/" "Archive""#,
                "\r\n",
                "{tag} OK LIST completed\r\n",
            )),
            Step::Expect(r#"a4 EXAMINE "INBOX""#),
            Step::Send(concat!(
                r"* FLAGS (\Seen \Flagged \Answered)",
                "\r\n",
                "* 2 EXISTS\r\n",
                "* OK [UIDVALIDITY 7] ok\r\n",
                "* OK [UIDNEXT 12] ok\r\n",
                "{tag} OK [READ-ONLY] EXAMINE completed\r\n",
            )),
            Step::Expect("a5 UID FETCH 1:* (UID FLAGS)"),
            Step::Send(concat!(
                r"* 1 FETCH (UID 10 FLAGS (\Seen))",
                "\r\n",
                r"* 2 FETCH (UID 11 FLAGS (\Flagged \Answered))",
                "\r\n",
                "{tag} OK FETCH completed\r\n",
            )),
            Step::Expect("a6 LOGOUT"),
            Step::Send("* BYE bye\r\n{tag} OK LOGOUT completed\r\n"),
            Step::Close,
        ]);

        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        assert!(imap.has_capability("move") && !imap.has_capability("UIDPLUS"));
        imap.set_read_only(true);
        let folders = imap.list_folders().unwrap();
        let names: Vec<_> = folders.iter().map(|f| (f.name.as_str(), f.role)).collect();
        assert_eq!(names, [("INBOX", Role::Inbox), ("Gesendet", Role::Sent)]);
        let sel = imap.select("INBOX").unwrap();
        assert_eq!((sel.uidvalidity, sel.exists), (7, 2));
        let flags = imap.uid_flags().unwrap();
        assert_eq!(
            flags,
            HashMap::from([(10, db::SEEN), (11, db::FLAGGED | db::ANSWERED)])
        );
        assert_eq!(imap.commands(), 5);
        assert!(imap.bytes() > 0);
        assert_eq!(imap.socket().read_timeout().unwrap(), Some(IO_TIMEOUT));
        imap.logout();

        let sent = server.finish();
        assert_eq!(sent.len(), 6);
        assert!(sent.iter().all(|l| !l.contains("SELECT")));
    }

    #[test]
    fn acting_on_a_uid_newer_than_the_selection_reselects_first() {
        // Postale's Dovecot never shows a long-open selection mail delivered
        // later, and a STORE on a UID it doesn't know silently does nothing.
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(select_reply(3, 12)),
            // UID 11 is below the UIDNEXT seen at SELECT: no new SELECT.
            Step::Expect(r"a4 UID STORE 11 +FLAGS.SILENT (\Seen)"),
            Step::Send("{tag} OK STORE completed\r\n"),
            // UID 15 is newer: select again first.
            Step::Expect(r#"a5 SELECT "INBOX""#),
            Step::Send(select_reply(4, 16)),
            Step::Expect(r"a6 UID STORE 15 +FLAGS.SILENT (\Seen)"),
            Step::Send("{tag} OK STORE completed\r\n"),
        ]);
        script.extend(logout_steps("a7 LOGOUT"));
        let server = FakeServer::start(script);

        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        imap.select("INBOX").unwrap();
        imap.set_flags(("INBOX", 7), &[11], "\\Seen", true).unwrap();
        imap.set_flags(("INBOX", 7), &[15], "\\Seen", true).unwrap();
        imap.logout();
        server.finish();
    }

    #[test]
    fn idle_deadline_holds_despite_keepalives() {
        // Dovecot sends `* OK Still here` while idling; the crate restarts
        // its per-read timeout on each, so only our deadline ends the wait.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            for _ in 0..30 {
                if s.write_all(b"* OK Still here\r\n").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let mut stream = Counted::new(
            Transport::Plain(TcpStream::connect(addr).unwrap()),
            Arc::default(),
            Arc::default(),
        );
        let start = Instant::now();
        stream
            .set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let mut buf = [0u8; 64];
        let err = loop {
            match stream.read(&mut buf) {
                Ok(n) => assert!(n > 0, "server closed early"),
                Err(e) => break e,
            }
        };
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        assert!(
            start.elapsed() < Duration::from_millis(1500),
            "{:?}",
            start.elapsed()
        );
        // `None` (after the wait) clears the deadline: reads work again.
        stream.set_read_timeout(None).unwrap();
        assert!(stream.read(&mut buf).is_ok());
        drop(stream);
        server.join().unwrap();
    }

    #[test]
    fn list_status_leaves_the_selected_folder_first() {
        // While a folder is selected the server may push `* n FETCH` and
        // `* OK [...]` lines, which the crate's LIST parser rejects. Seen live
        // on Dovecot after new mail arrived; UNSELECT absorbs them safely.
        let mut script = login_steps(
            "* CAPABILITY IMAP4rev1 CONDSTORE LIST-EXTENDED LIST-STATUS UNSELECT\r\n{tag} OK done\r\n",
        );
        script.extend([
            Step::Expect(r#"a3 EXAMINE "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 UNSELECT"),
            Step::Send(concat!(
                r"* 3 FETCH (FLAGS (\Seen))",
                "\r\n* OK [HIGHESTMODSEQ 9] ok\r\n{tag} OK UNSELECT completed\r\n",
            )),
            Step::Expect(leak(format!("a5 {LIST_STATUS}"))),
            Step::Send(concat!(
                r#"* LIST (\HasNoChildren) "/" "INBOX""#,
                "\r\n",
                r#"* STATUS "INBOX" (MESSAGES 3 UIDNEXT 12 UIDVALIDITY 7 HIGHESTMODSEQ 9)"#,
                "\r\n{tag} OK LIST completed\r\n",
            )),
        ]);
        script.extend(logout_steps("a6 LOGOUT"));
        let server = FakeServer::start(script);

        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        imap.set_read_only(true);
        imap.select("INBOX").unwrap();
        let (folders, statuses) = imap.list_status().unwrap().unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(statuses["INBOX"].hms, 9);
        // Nothing selected now, so the next list_status sends no UNSELECT.
        assert!(!imap.in_folder);
        imap.logout();
        server.finish();
    }

    const IDLE_CAPS: &str = "* CAPABILITY IMAP4rev1 IDLE\r\n{tag} OK done\r\n";

    #[test]
    fn idle_reports_changes_and_renews_after_a_timeout() {
        let mut script = login_steps(IDLE_CAPS);
        script.extend([
            Step::Expect(r#"a3 EXAMINE "INBOX""#),
            Step::Send(SELECT_OK),
            // A keepalive is no change; the wait then times out.
            Step::Expect("a4 IDLE"),
            Step::Send("+ idling\r\n* OK Still here\r\n"),
            Step::Expect("DONE"),
            Step::Send("a4 OK IDLE terminated\r\n"),
            Step::Expect("a5 IDLE"),
            Step::Send("+ idling\r\n* 5 EXISTS\r\n"),
            Step::Expect("DONE"),
            Step::Send("* 5 RECENT\r\na5 OK IDLE terminated\r\n"),
        ]);
        script.extend(logout_steps("a6 LOGOUT"));
        let server = FakeServer::start(script);

        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        imap.set_read_only(true);
        imap.select("INBOX").unwrap();
        let wait = Duration::from_millis(300);
        assert_eq!(imap.idle(wait).unwrap(), WaitOutcome::TimedOut);
        // The crate leaves the socket without a timeout after a wait.
        assert_eq!(imap.socket().read_timeout().unwrap(), Some(IO_TIMEOUT));
        assert_eq!(imap.idle(wait).unwrap(), WaitOutcome::MailboxChanged);
        assert_eq!(imap.socket().read_timeout().unwrap(), Some(IO_TIMEOUT));
        imap.logout();
        server.finish();
    }

    #[test]
    fn refused_idle_is_an_error_and_sends_no_done() {
        let mut script = login_steps(IDLE_CAPS);
        script.extend([
            Step::Expect(r#"a3 EXAMINE "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 IDLE"),
            Step::Send("{tag} NO IDLE not allowed now\r\n"),
            // A DONE here would draw a `DONE BAD` the crate panics on.
            Step::ExpectEof,
        ]);
        let server = FakeServer::start(script);

        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        imap.set_read_only(true);
        imap.select("INBOX").unwrap();
        let err = imap.idle(Duration::from_secs(5)).err().unwrap();
        assert!(matches!(
            err.downcast_ref::<imap::Error>(),
            Some(imap::Error::No(_))
        ));
        // Even LOGOUT can't flush the queued DONE out.
        imap.logout();
        assert_eq!(server.finish().len(), 4);
    }

    fn has_expunge(sent: &[String]) -> bool {
        sent.iter().any(|l| l.split(' ').nth(1) == Some("EXPUNGE"))
    }

    #[test]
    fn delete_without_uidplus_leaves_message_flagged() {
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect(r"a4 UID STORE 5 +FLAGS.SILENT (\Deleted)"),
            Step::Send("{tag} OK STORE completed\r\n"),
        ]);
        script.extend(logout_steps("a5 LOGOUT"));
        let server = FakeServer::start(script);

        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let done = imap.delete_messages(("INBOX", 7), &[5]).unwrap();
        assert_eq!(done, Expunged::LeftFlagged);
        imap.logout();
        assert!(!has_expunge(&server.finish()));
    }

    #[test]
    fn copy_move_quotes_the_name_and_never_expunges() {
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            // A failed COPY flags nothing.
            Step::Expect(r#"a4 UID COPY 5 "Gone""#),
            Step::Send("{tag} NO [TRYCREATE] no such folder\r\n"),
            Step::Expect(r#"a5 UID COPY 5 "Sent Items""#),
            Step::Send("{tag} OK COPY completed\r\n"),
            Step::Expect(r"a6 UID STORE 5 +FLAGS.SILENT (\Deleted)"),
            Step::Send("{tag} OK STORE completed\r\n"),
        ]);
        script.extend(logout_steps("a7 LOGOUT"));
        let server = FakeServer::start(script);

        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        assert!(imap.move_messages(("INBOX", 7), &[5], "Gone").is_err());
        let done = imap
            .move_messages(("INBOX", 7), &[5], "Sent Items")
            .unwrap();
        assert_eq!(done, Expunged::LeftFlagged);
        // Names the crate would send unescaped are refused before any I/O.
        assert!(imap.append("Bad\"Name", b"x").is_err());
        assert!(imap.move_messages(("INBOX", 7), &[5], "a\r\nb").is_err());
        imap.logout();
        assert!(!has_expunge(&server.finish()));
    }

    #[test]
    fn failed_select_forgets_the_previous_folder() {
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect(r#"a4 SELECT "Gone""#),
            Step::Send("{tag} NO no such folder\r\n"),
            Step::Expect(r#"a5 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect(r"a6 UID STORE 5 +FLAGS.SILENT (\Seen)"),
            Step::Send("{tag} OK STORE completed\r\n"),
        ]);
        script.extend(logout_steps("a7 LOGOUT"));
        let server = FakeServer::start(script);

        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let sel = imap.select("INBOX").unwrap();
        assert_eq!((sel.uidvalidity, sel.exists, sel.uid_next), (7, 3, 12));
        assert!(imap.select("Gone").is_err());
        imap.set_flags(("INBOX", 7), &[5], "\\Seen", true).unwrap();
        imap.logout();
        server.finish();
    }

    #[test]
    fn header_dates_fall_back_and_deleted_is_skipped() {
        let reply = [
            header_fetch(1, 1, "", "Subject: no date\r\n\r\n"),
            header_fetch(2, 2, "", "Date: Sat, 1 Jan 2050 00:00:00 +0000\r\n\r\n"),
            header_fetch(
                3,
                3,
                r"\Seen",
                "Date: Sun, 31 Dec 2023 12:00:00 +0000\r\n\r\n",
            ),
            header_fetch(4, 4, r"\Deleted", "Subject: gone\r\n\r\n"),
        ]
        .concat()
            + "{tag} OK FETCH completed\r\n";
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 UID FETCH 1:4 (UID FLAGS INTERNALDATE RFC822.SIZE"),
            Step::Send(leak(reply)),
        ]);
        script.extend(logout_steps("a5 LOGOUT"));
        let server = FakeServer::start(script);

        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        imap.select("INBOX").unwrap();
        let headers = imap.fetch_headers(&[1, 2, 3, 4]).unwrap();
        let dates: Vec<_> = headers.iter().map(|h| (h.uid, h.date)).collect();
        assert_eq!(dates, [(1, 1704067200), (2, 1704067200), (3, 1704024000)]);
        assert!(headers[2].seen);
        imap.logout();
        server.finish();
    }

    #[test]
    fn dates_and_quoting() {
        assert_eq!(message_date(0, Some(100)), 100);
        assert_eq!(message_date(100 + 86_401, Some(100)), 100);
        assert_eq!(message_date(100 + 3_600, Some(100)), 3_700);
        assert_eq!(message_date(50, None), 50);
        assert_eq!(quote("Sent Items").unwrap(), r#""Sent Items""#);
        assert_eq!(quote(r#"a"b\c"#).unwrap(), r#""a\"b\\c""#);
        assert!(quote("a\0b").is_err());
        assert!(reads_only("UID FETCH 1:* (UID) (CHANGEDSINCE 1)"));
        assert!(reads_only("noop"));
        for writes in [
            r"UID STORE 1 +FLAGS (\Deleted)",
            "EXPUNGE",
            "DELETE x",
            "NOOP\r\nEXPUNGE",
        ] {
            assert!(!reads_only(writes), "{writes:?}");
        }
    }

    #[test]
    fn folds_status_responses_by_folder() {
        let status = |mailbox: &str, attributes| UnsolicitedResponse::Status {
            mailbox: mailbox.to_string(),
            attributes,
        };
        let got = fold_statuses(
            [
                status(
                    "INBOX",
                    vec![
                        StatusAttribute::Messages(3),
                        StatusAttribute::UidNext(12),
                        StatusAttribute::UidValidity(7),
                        StatusAttribute::HighestModSeq(1 << 40),
                        StatusAttribute::Unseen(1),
                    ],
                ),
                UnsolicitedResponse::Exists(9),
                UnsolicitedResponse::Expunge(1),
                // No HIGHESTMODSEQ (NOMODSEQ): left at 0.
                status("Sent", vec![StatusAttribute::Messages(2)]),
            ]
            .into_iter(),
        );
        assert_eq!(
            got,
            HashMap::from([
                (
                    "INBOX".to_string(),
                    FolderStatus {
                        messages: 3,
                        uidnext: 12,
                        uidvalidity: 7,
                        hms: 1 << 40,
                    }
                ),
                (
                    "Sent".to_string(),
                    FolderStatus {
                        messages: 2,
                        ..Default::default()
                    }
                ),
            ])
        );
    }

    #[test]
    fn packs_flags() {
        assert_eq!(
            flag_bits(&[Flag::Seen, Flag::Answered, Flag::Deleted]),
            db::SEEN | db::ANSWERED | db::DELETED
        );
        assert_eq!(flag_bits(&[]), 0);
    }

    #[test]
    fn compresses_uid_sets() {
        assert_eq!(uid_set(&[5, 1, 2, 3, 9, 10]), "1:3,5,9:10");
        assert_eq!(uid_set(&[7]), "7");
        assert_eq!(uid_set(&[12, 1, 2, 3, 4, 5, 7, 9, 10, 11, 7]), "1:5,7,9:12");
        assert_eq!(uid_set(&[]), "");
        // Long selections split into sets that each fit a command line.
        assert_eq!(
            uid_sets(&[1, 2, 3, 5, 7, 9, 10, 11, 12], 7),
            ["1:3,5,7", "9:12"]
        );
        let sparse: Vec<u32> = (0..5000).map(|n| n * 2 + 1).collect();
        let sets = uid_sets(&sparse, MAX_UID_SET);
        assert!(sets.len() > 1 && sets.iter().all(|s| s.len() <= MAX_UID_SET));
        assert_eq!(sets.join(","), uid_set(&sparse));
        assert_eq!(
            uid_set(&[u32::MAX, 1, u32::MAX - 1]),
            "1,4294967294:4294967295"
        );
    }

    #[test]
    fn folder_roles_and_names() {
        let f = remote_folder("INBOX", Some("/"), None);
        assert_eq!((f.display.as_str(), f.role), ("Inbox", Role::Inbox));
        let f = remote_folder("INBOX.Sent Items", Some("."), None);
        assert_eq!(
            (f.display.as_str(), f.role, f.depth),
            ("Sent Items", Role::Sent, 0)
        );
        let f = remote_folder("Projects/Entw&APw-rfe", Some("/"), None);
        assert_eq!(
            (f.display.as_str(), f.role, f.depth),
            ("Entwürfe", Role::Other, 1)
        );
        let f = remote_folder("Papierkorb", Some("/"), Some(Role::Trash));
        assert_eq!(f.role, Role::Trash);
        let f = remote_folder("INBOX", Some("/"), Some(Role::Archive));
        assert_eq!(f.role, Role::Inbox);
    }

    #[test]
    fn modified_utf7() {
        assert_eq!(decode_mutf7("Hello"), "Hello");
        assert_eq!(decode_mutf7("&ZeVnLIqe-"), "日本語");
        assert_eq!(decode_mutf7("A&-B"), "A&B");
    }

    #[test]
    fn invalid_modified_utf7_is_kept_as_the_server_wrote_it() {
        for raw in [
            "&",             // nothing after it
            "&abc",          // no closing dash
            "&AB-",          // half a UTF-16 unit
            "&!!-",          // not base64
            "&2D4-",         // a lone surrogate
            "&AAA=-",        // padding is not part of it
            "Mail&ZeVnLIqe", // unterminated after valid text
        ] {
            assert_eq!(decode_mutf7(raw), raw, "{raw}");
        }
        // Valid and invalid in one name: only the valid one is decoded.
        assert_eq!(decode_mutf7("&ZeVnLIqe-/&AB-"), "日本語/&AB-");
    }

    #[test]
    fn folder_names_of_every_style() {
        let f = |name: &str, delim: &str| remote_folder(name, Some(delim), None);
        // Case variants of INBOX are the inbox, shown as "Inbox".
        for name in ["INBOX", "Inbox", "inbox"] {
            let r = f(name, "/");
            assert_eq!((r.role, r.display.as_str()), (Role::Inbox, "Inbox"));
        }
        // Courier and Cyrus: `INBOX.` before the name, `.` between levels.
        for (name, role) in [
            ("INBOX.Sent", Role::Sent),
            ("INBOX.Sent Items", Role::Sent),
            ("INBOX.Sent Messages", Role::Sent),
            ("INBOX.Drafts", Role::Drafts),
            ("INBOX.Deleted Items", Role::Trash),
            ("INBOX.Deleted Messages", Role::Trash),
            ("INBOX.Spam", Role::Junk),
            ("INBOX.Junk E-mail", Role::Junk),
            ("INBOX.Archives", Role::Archive),
            ("inbox.trash", Role::Trash),
        ] {
            assert_eq!(f(name, ".").role, role, "{name}");
        }
        // The same with a `/`, with and without the prefix.
        assert_eq!(f("INBOX/Sent", "/").role, Role::Sent);
        assert_eq!(f("Sent Messages", "/").role, Role::Sent);
        // A folder of the user's own, deeper down, keeps its place and name.
        let nested = f("INBOX.Projects.Archive", ".");
        assert_eq!(
            (nested.role, nested.display.as_str(), nested.depth),
            (Role::Other, "Archive", 2)
        );
        let nested = f("Work/Sent", "/");
        assert_eq!((nested.role, nested.depth), (Role::Other, 1));
        // The delimiter is the one LIST reported: a `/` in a `.` name is text.
        let odd = f("INBOX.a/b", ".");
        assert_eq!((odd.display.as_str(), odd.depth), ("a/b", 1));
        // Quotes, backslashes, `&`, spaces: shown and kept verbatim.
        let odd = f("Q\"uote \\ & Co", "/");
        assert_eq!(
            (odd.name.as_str(), odd.display.as_str()),
            ("Q\"uote \\ & Co", "Q\"uote \\ & Co")
        );
        // Very long names are shortened for display, never for commands.
        let long = "x".repeat(500);
        let r = f(&long, "/");
        assert_eq!(r.name, long);
        assert_eq!(r.display.chars().count(), MAX_DISPLAY);
        assert!(r.display.ends_with('\u{2026}'));
        // No delimiter reported (a flat server): the whole name, no depth.
        let flat = remote_folder("Archive.2024", None, None);
        assert_eq!((flat.display.as_str(), flat.depth), ("Archive.2024", 0));
    }

    #[test]
    fn unselectable_names_are_not_folders() {
        let server = FakeServer::start({
            let mut script = login_steps(PLAIN_CAPS);
            script.extend([
                Step::Expect("a3 LIST"),
                Step::Send(concat!(
                    r#"* LIST (\HasNoChildren) "." "INBOX""#,
                    "\r\n",
                    r#"* LIST (\Noselect \HasChildren) "." "INBOX.Projects""#,
                    "\r\n",
                    r#"* LIST (\NonExistent \HasChildren) "." "INBOX.Ghost""#,
                    "\r\n",
                    r#"* LIST (\HasNoChildren) "." "INBOX.Real""#,
                    "\r\n",
                    "{tag} OK LIST completed\r\n",
                )),
            ]);
            script.extend(logout_steps("a4 LOGOUT"));
            script
        });
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let names: Vec<_> = imap
            .list_folders()
            .unwrap()
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(names, ["INBOX", "INBOX.Real"]);
        imap.logout();
        server.finish();
    }

    // ---- greetings that are no greeting

    #[test]
    fn preauth_bye_and_silence_are_not_a_greeting() {
        for (greeting, want) in [
            ("* PREAUTH welcome back\r\n", Some(GreetingRefused::PreAuth)),
            ("* BYE too many users\r\n", Some(GreetingRefused::Bye)),
            ("HTTP/1.1 400 Bad Request\r\n", Some(GreetingRefused::Other)),
            ("", None),
        ] {
            let server = FakeServer::start(if greeting.is_empty() {
                vec![Step::Close]
            } else {
                vec![Step::Send(greeting), Step::ExpectEof]
            });
            let err = Imap::connect_plain(server.addr, "user", "pass")
                .err()
                .expect("refused");
            match want {
                Some(want) => assert_eq!(err.downcast_ref::<GreetingRefused>(), Some(&want)),
                None => assert!(err.is::<ConnectionClosed>(), "{err:#}"),
            }
            // No LOGIN was sent to it.
            assert!(server.finish().is_empty());
        }
    }

    // ---- the sign-in: bytes, and the methods

    /// The LOGIN line the fake got for `password`.
    fn login_line_for(password: &str) -> String {
        let server = FakeServer::start({
            let mut script = login_steps(PLAIN_CAPS);
            script.extend(logout_steps("a3 LOGOUT"));
            script
        });
        Imap::connect_plain(server.addr, "alice", password)
            .unwrap()
            .logout();
        server.finish().remove(0)
    }

    #[test]
    fn passwords_are_sent_as_typed() {
        // Quotes and backslashes escaped; spaces and non-ASCII (UTF-8) as is.
        assert_eq!(
            login_line_for(r#"p"w\ x"#),
            r#"a1 LOGIN "alice" "p\"w\\ x""#
        );
        assert_eq!(
            login_line_for("pässwörd ✓"),
            "a1 LOGIN \"alice\" \"pässwörd ✓\""
        );
    }

    /// A connection that asks for the capabilities before the sign-in (as
    /// after STARTTLS), given the server's `caps` line; the fake's script
    /// after that is `then`.
    fn listed_sign_in(caps: &'static str, then: Vec<Step>) -> Result<Vec<String>> {
        let mut script = vec![
            Step::Send("* OK fake IMAP ready\r\n"),
            Step::Expect("c0 CAPABILITY"),
            Step::Send(caps),
        ];
        script.extend(then);
        let server = FakeServer::start(script);
        // Dropped (closed) before the script is read to its end.
        let result = Imap::connect_plain_listing(server.addr, "alice", "pw").map(drop);
        let seen = server.finish();
        match result {
            Ok(()) => Ok(seen),
            Err(e) => Err(e.context(seen.join("|"))),
        }
    }

    #[test]
    fn a_server_that_disables_login_gets_nothing_at_all() {
        // The profile of LOGINDISABLED under TLS (AUTH=PLAIN or not): the
        // password is never sent, and the error says why.
        for caps in [
            "* CAPABILITY IMAP4rev1 LOGINDISABLED\r\nc0 OK done\r\n",
            "* CAPABILITY IMAP4rev1 LOGINDISABLED AUTH=PLAIN\r\nc0 OK done\r\n",
        ] {
            let err = listed_sign_in(caps, vec![Step::ExpectEof]).unwrap_err();
            assert!(err.chain().any(|c| c.is::<NoSignIn>()), "{err:#}");
        }
        // Without it the listing is followed by an ordinary LOGIN.
        let then = vec![
            Step::Expect(r#"a1 LOGIN "alice" "pw""#),
            Step::Send("{tag} OK LOGIN completed\r\n"),
            Step::Expect("a2 CAPABILITY"),
            Step::Send(PLAIN_CAPS),
            Step::ExpectEof,
        ];
        let seen =
            listed_sign_in("* CAPABILITY IMAP4rev1 AUTH=PLAIN\r\nc0 OK done\r\n", then).unwrap();
        assert_eq!(seen[0], "c0 CAPABILITY");
    }

    #[test]
    fn a_greeting_that_lists_the_capabilities_saves_the_round_trip() {
        // Dovecot's greeting: LOGINDISABLED there means no LOGIN, no ask.
        let server = FakeServer::start(vec![
            Step::Send("* OK [CAPABILITY IMAP4rev1 LOGINDISABLED] ready\r\n"),
            Step::ExpectEof,
        ]);
        let err = Imap::connect_plain(server.addr, "alice", "pw")
            .err()
            .expect("refused");
        assert!(err.is::<NoSignIn>(), "{err:#}");
        assert!(server.finish().is_empty());
    }

    #[test]
    fn utf8_only_servers_are_refused_with_a_sentence_of_their_own() {
        let mut script = login_steps("* CAPABILITY IMAP4rev1 UTF8=ONLY\r\n{tag} OK done\r\n");
        script.push(Step::ExpectEof);
        let server = FakeServer::start(script);
        let err = Imap::connect_plain(server.addr, "alice", "pw")
            .err()
            .expect("refused");
        assert!(err.is::<Utf8Only>(), "{err:#}");
        server.finish();
    }

    #[test]
    fn login_refusals_by_response_code() {
        assert!(rejects_credentials(
            "[AUTHENTICATIONFAILED] Invalid credentials"
        ));
        assert!(rejects_credentials("[EXPIRED] Password expired"));
        assert!(rejects_credentials("LOGIN failed."));
        assert!(!rejects_credentials("[UNAVAILABLE] Try again later"));
        assert!(!rejects_credentials("[INUSE] Mailbox in use"));
        assert!(!rejects_credentials("[LIMIT] Too many connections"));
    }

    #[test]
    fn login_no_is_typed() {
        let server = FakeServer::start(vec![
            Step::Send("* OK fake IMAP ready\r\n"),
            Step::Expect("a1 LOGIN"),
            Step::Send("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n"),
        ]);
        let err = Imap::connect_plain(server.addr, "user", "pass")
            .err()
            .unwrap();
        server.finish();
        assert!(err.downcast_ref::<AuthFailed>().is_some());
    }

    #[test]
    fn interleaves_address_families() {
        let v4 = |n| SocketAddr::from(([10, 0, 0, n], 993));
        let v6 = |n| SocketAddr::from(([0xfd00, 0, 0, 0, 0, 0, 0, n], 993));
        assert_eq!(
            interleave(vec![v6(1), v6(2), v6(3), v4(1)]),
            [v6(1), v4(1), v6(2), v6(3)]
        );
        assert_eq!(
            interleave(vec![v4(1), v4(2), v6(1), v6(2)]),
            [v4(1), v6(1), v4(2), v6(2)]
        );
        assert!(interleave(Vec::new()).is_empty());
    }

    #[test]
    fn connect_any_moves_past_a_dead_address() {
        // A port nothing listens on any more, then a live one.
        let dead = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let live = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let tcp = connect_any(&[dead, live.local_addr().unwrap()]).unwrap();
        assert_eq!(tcp.peer_addr().unwrap(), live.local_addr().unwrap());
        assert!(tcp.nodelay().unwrap());
        assert!(connect_any(&[dead]).is_err());
    }

    /// A stream that replays canned server bytes and swallows writes.
    struct Canned(std::io::Cursor<&'static [u8]>);

    impl Read for Canned {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Write for Canned {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn starttls_refuses_bytes_before_the_handshake() {
        let canned = |s: &'static str| Canned(std::io::Cursor::new(s.as_bytes()));
        const READY: &str =
            "* OK ready\r\n* CAPABILITY IMAP4rev1 STARTTLS LOGINDISABLED\r\na0 OK done\r\n";
        assert!(starttls(canned(leak(format!("{READY}a1 OK begin TLS\r\n")))).is_ok());
        let err = starttls(canned(leak(format!(
            "{READY}a1 OK begin TLS\r\n* OK injected\r\n"
        ))))
        .err()
        .unwrap();
        assert!(err.to_string().contains("unexpected data"));
        assert!(starttls(canned(leak(format!("{READY}a1 NO not now\r\n")))).is_err());
        // A server that doesn't list STARTTLS is never asked for it.
        let err = starttls(canned(
            "* OK ready\r\n* CAPABILITY IMAP4rev1\r\na0 OK done\r\na1 OK begin TLS\r\n",
        ))
        .err()
        .unwrap();
        assert!(err.is::<StartTlsRefused>(), "{err:#}");
        // PREAUTH and BYE are no greeting.
        for greeting in ["* PREAUTH hi\r\n", "* BYE go away\r\n"] {
            let err = starttls(canned(greeting)).err().unwrap();
            assert!(err.is::<GreetingRefused>(), "{err:#}");
        }
    }

    // ---- the fence: replies the crate would abort on, and odd servers

    /// A session after LOGIN with `caps`, then `steps`, then LOGOUT as
    /// command `logout` (none if the connection is expected to fail).
    fn session(steps: Vec<Step>, logout: Option<&'static str>) -> (FakeServer, Imap) {
        let mut script = login_steps(PLAIN_CAPS);
        script.extend(steps);
        match logout {
            Some(line) => script.extend(logout_steps(line)),
            None => script.push(Step::ExpectEof),
        }
        let server = FakeServer::start(script);
        let imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        (server, imap)
    }

    #[test]
    fn a_tagged_ok_to_idle_without_continuation_fails_the_connection() {
        let mut script = login_steps(IDLE_CAPS);
        script.extend([
            Step::Expect(r#"a3 EXAMINE "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 IDLE"),
            // The crate would reach `unreachable!()` on this.
            Step::Send("{tag} OK IDLE done\r\n"),
            Step::ExpectEof,
        ]);
        let server = FakeServer::start(script);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        imap.set_read_only(true);
        imap.select("INBOX").unwrap();
        let err = imap.idle(Duration::from_secs(5)).err().unwrap();
        assert_eq!(failure(&err), Some(Failure::Protocol));
        drop(imap);
        // No DONE (and nothing else) went out.
        assert_eq!(server.finish().len(), 4);
    }

    #[test]
    fn a_reply_with_another_commands_tag_fails_instead_of_aborting() {
        // The crate asserts the tag matches, which aborts a release build.
        let (server, mut imap) = session(
            vec![Step::Expect("a3 NOOP"), Step::Send("a9 OK NOOP done\r\n")],
            None,
        );
        let err = imap.noop().err().unwrap();
        assert_eq!(failure(&err), Some(Failure::Protocol));
        drop(imap);
        server.finish();
        // Dovecot's answer to a stray DONE is tagged `DONE`.
        let (server, mut imap) = session(
            vec![
                Step::Expect("a3 NOOP"),
                Step::Send("DONE BAD expected IDLE\r\n"),
            ],
            None,
        );
        assert_eq!(
            failure(&imap.noop().err().unwrap()),
            Some(Failure::Protocol)
        );
        drop(imap);
        server.finish();
    }

    #[test]
    fn lines_inside_a_literal_are_message_text() {
        // A body whose lines look like tagged and untagged replies.
        let raw = "Subject: x\r\n\r\na9 OK not a reply\r\n* BYE not either\r\n+ nor this {12}\r\n";
        let reply = leak(format!(
            "* 1 FETCH (UID 5 BODY[] {{{}}}\r\n{raw})\r\n{{tag}} OK done\r\n",
            raw.len()
        ));
        let (server, mut imap) = session(
            vec![
                Step::Expect(r#"a3 SELECT "INBOX""#),
                Step::Send(SELECT_OK),
                Step::Expect("a4 UID FETCH 5 (UID BODY.PEEK[])"),
                Step::Send(reply),
            ],
            Some("a5 LOGOUT"),
        );
        let bodies = imap.fetch_bodies(("INBOX", 7), &[5]).unwrap();
        assert_eq!(bodies, [(5, raw.as_bytes().to_vec())]);
        imap.logout();
        server.finish();
    }

    #[test]
    fn an_expunge_during_a_fetch_is_set_aside() {
        let (server, mut imap) = session(
            vec![
                Step::Expect(r#"a3 SELECT "INBOX""#),
                Step::Send(SELECT_OK),
                Step::Expect("a4 UID FETCH 1:* (UID FLAGS)"),
                Step::Send(concat!(
                    "* 1 FETCH (UID 10 FLAGS (\\Seen))\r\n",
                    "* 2 EXPUNGE\r\n",
                    "* 2 FETCH (UID 12 FLAGS ())\r\n",
                    "{tag} OK done\r\n",
                )),
            ],
            Some("a5 LOGOUT"),
        );
        imap.select("INBOX").unwrap();
        let flags = imap.uid_flags().unwrap();
        assert_eq!(flags, HashMap::from([(10, db::SEEN), (12, 0)]));
        imap.drain_unsolicited();
        imap.logout();
        server.finish();
    }

    #[test]
    fn an_unsolicited_ok_with_a_code_during_a_fetch() {
        let (server, mut imap) = session(
            vec![
                Step::Expect(r#"a3 SELECT "INBOX""#),
                Step::Send(SELECT_OK),
                Step::Expect("a4 UID FETCH 1:* (UID FLAGS)"),
                Step::Send(concat!(
                    "* OK [HIGHESTMODSEQ 99] changed\r\n",
                    "* 1 FETCH (UID 10 FLAGS ())\r\n",
                    "* OK [ALERT] the server says hello\r\n",
                    "{tag} OK done\r\n",
                )),
            ],
            Some("a5 LOGOUT"),
        );
        imap.select("INBOX").unwrap();
        // Whatever the crate makes of it: no panic, and the stream stays
        // usable or the error says it is not.
        match imap.uid_flags() {
            Ok(flags) => {
                assert_eq!(flags, HashMap::from([(10, 0)]));
                imap.logout();
                server.finish();
            }
            Err(e) => assert_eq!(failure(&e), Some(Failure::Protocol)),
        }
    }

    #[test]
    fn a_truncated_literal_is_a_lost_connection() {
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 UID FETCH 5 (UID BODY.PEEK[])"),
            Step::Send("* 1 FETCH (UID 5 BODY[] {100}\r\nSubject: cut off"),
            Step::Close,
        ]);
        let server = FakeServer::start(script);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let err = imap.fetch_bodies(("INBOX", 7), &[5]).err().unwrap();
        assert_eq!(failure(&err), Some(Failure::Unreachable));
        server.finish();
    }

    #[test]
    fn esearch_and_vanished_replies_fail_without_a_panic() {
        // Never asked for, but a server may send them anyway.
        let (server, mut imap) = session(
            vec![
                Step::Expect(r#"a3 SELECT "INBOX""#),
                Step::Send(SELECT_OK),
                Step::Expect("a4 UID SEARCH ALL"),
                Step::Send("* ESEARCH (TAG \"a4\") UID ALL 1:3\r\n{tag} OK done\r\n"),
            ],
            None,
        );
        imap.select("INBOX").unwrap();
        match imap.uids() {
            // Read as no match: the delta's EXISTS check refuses an answer
            // that disagrees with the folder's count.
            Ok(uids) => assert!(uids.is_empty()),
            Err(e) => assert_eq!(failure(&e), Some(Failure::Protocol)),
        }
        drop(imap);
        server.finish();

        let (server, mut imap) = session(
            vec![
                Step::Expect(r#"a3 SELECT "INBOX""#),
                Step::Send(SELECT_OK),
                Step::Expect("a4 UID FETCH 1:* (UID FLAGS)"),
                Step::Send("* VANISHED (EARLIER) 1:2\r\n{tag} OK done\r\n"),
            ],
            None,
        );
        imap.select("INBOX").unwrap();
        let err = imap.uid_flags().err().unwrap();
        assert_eq!(failure(&err), Some(Failure::Protocol));
        drop(imap);
        server.finish();
    }

    #[test]
    fn literal_announcements() {
        assert_eq!(announced(b"* 1 FETCH (BODY[] {123}\r\n"), 123);
        assert_eq!(announced(b"a5 APPEND \"x\" () {9+}\r\n"), 9);
        assert_eq!(announced(b"* 1 FETCH (BINARY[] ~{7}\r\n"), 7);
        assert_eq!(announced(b"* OK {not a literal}\r\n"), 0);
        assert_eq!(announced(b"{}\r\n"), 0);
        assert_eq!(announced(b"* OK done\r\n"), 0);
        assert_eq!(announced(b"{99999999999999999999999}\r\n"), u64::MAX);
        // Lines are found across reads, and literals passed over.
        let mut lines = Lines::default();
        let mut heads = Vec::new();
        for part in [
            &b"* 1 FETCH (BODY[] {5}\r"[..],
            b"\na1 OK",
            b")\r\na2 OK done\r\n",
        ] {
            lines
                .feed(part, |h| {
                    heads.push(String::from_utf8_lossy(h).into_owned());
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(heads, ["* 1 FETCH (BODY[] {5}\r\n", "a2 OK done\r\n"]);
    }

    /// Whether Rust source names the crate: `imap_proto`, a crate-rooted
    /// `::imap::`, or one of the crate's own items. Our module's paths
    /// (`mail::imap::Imap`, `super::imap::failure`) don't count.
    fn names_crate(text: &str) -> bool {
        let rooted = text.match_indices("::imap::").any(|(i, _)| {
            !text[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        });
        rooted
            || [
                "imap_proto",
                "imap::Error",
                "imap::types",
                "imap::extensions",
                "imap::Session",
            ]
            .iter()
            .any(|n| text.contains(n))
    }

    /// Only this file names the `imap` crate (and imap-proto); everything
    /// else goes through [`Imap`] and [`failure`].
    #[test]
    fn only_this_file_names_the_imap_crate() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut stack = vec![root.join("src"), root.join("examples"), root.join("tests")];
        let mut offenders = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && !path.ends_with("src/mail/imap.rs")
                    && names_crate(&std::fs::read_to_string(&path).unwrap())
                {
                    offenders.push(path.display().to_string());
                }
            }
        }
        assert!(offenders.is_empty(), "{offenders:?}");
        assert!(names_crate("let e = ::imap::Error::Bad(x);"));
        assert!(names_crate("use imap_proto::types::Capability;"));
        assert!(!names_crate(
            "use komorebi::mail::imap::{Imap, WaitOutcome};"
        ));
        assert!(!names_crate("use super::imap::{self, Failure};"));
    }
}
