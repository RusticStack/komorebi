//! Fakes for tests: a scripted IMAP server (a local `TcpListener` thread
//! that plays a fixed conversation and records every line the client sent)
//! and a small SMTP server. `Step::Send` writes its bytes as they are, so a
//! script can also play what real servers send unasked or wrongly
//! (unsolicited `* OK [...]` and EXPUNGE lines, ESEARCH, VANISHED, a
//! literal cut short, a reply with the wrong tag).

use super::mime::HEADER_FIELDS;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

pub enum Step {
    /// Read one line from the client; it must start with this prefix.
    Expect(&'static str),
    /// Send these bytes as-is, except that `{tag}` becomes the tag of the
    /// last line received (so replies need not hardcode `a1`, `a2`, ...).
    Send(&'static str),
    /// Run this (e.g. change the database) before the next step, to place
    /// it exactly between two commands.
    Then(Box<dyn FnOnce() + Send>),
    /// Read the literal announced at the end of the last line received
    /// (`{n}`, after a `+` reply) and record it as one entry.
    Literal,
    /// Close the connection.
    Close,
    /// The client must close the connection without sending anything more.
    ExpectEof,
}

pub struct FakeServer {
    pub addr: SocketAddr,
    handle: JoinHandle<Result<Vec<String>, String>>,
}

impl FakeServer {
    /// Listen on a free loopback port and play `script` to the first client.
    pub fn start(script: Vec<Step>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("binding the fake server");
        let addr = listener.local_addr().expect("fake server address");
        let handle = thread::spawn(move || run(listener, script));
        Self { addr, handle }
    }

    /// Wait for the script to finish and return the lines the client sent
    /// (without CRLF). Panics if the client strayed from the script.
    pub fn finish(self) -> Vec<String> {
        match self.handle.join().expect("fake server thread panicked") {
            Ok(lines) => lines,
            Err(e) => panic!("fake server: {e}"),
        }
    }
}

fn run(listener: TcpListener, script: Vec<Step>) -> Result<Vec<String>, String> {
    let (mut stream, _) = listener.accept().map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut received = Vec::new();
    let mut tag = String::new();
    for step in script {
        match step {
            Step::Expect(prefix) => {
                let mut line = String::new();
                if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                    return Err(format!("connection closed, expected {prefix:?}"));
                }
                let line = line.trim_end_matches(['\r', '\n']).to_string();
                if !line.starts_with(prefix) {
                    return Err(format!("expected {prefix:?}, got {line:?}"));
                }
                tag = line.split(' ').next().unwrap_or_default().to_string();
                received.push(line);
            }
            Step::Send(text) => stream
                .write_all(text.replace("{tag}", &tag).as_bytes())
                .map_err(|e| e.to_string())?,
            Step::Then(f) => f(),
            Step::Literal => {
                let last = received.last().map_or("", String::as_str);
                let n = last
                    .rsplit_once('{')
                    .and_then(|(_, n)| n.strip_suffix('}')?.parse::<usize>().ok())
                    .ok_or_else(|| format!("no literal announced in {last:?}"))?;
                let mut data = vec![0; n + 2];
                reader.read_exact(&mut data).map_err(|e| e.to_string())?;
                if !data.ends_with(b"\r\n") {
                    return Err("the literal is not followed by CRLF".into());
                }
                data.truncate(n);
                received.push(String::from_utf8_lossy(&data).into_owned());
            }
            Step::Close => {
                stream.shutdown(Shutdown::Both).ok();
                break;
            }
            Step::ExpectEof => {
                let mut line = String::new();
                if reader.read_line(&mut line).map_err(|e| e.to_string())? != 0 {
                    return Err(format!("expected the connection to close, got {line:?}"));
                }
            }
        }
    }
    Ok(received)
}

/// Greeting, LOGIN and CAPABILITY, advertising `caps`.
pub fn login_steps(caps: &'static str) -> Vec<Step> {
    vec![
        Step::Send("* OK fake IMAP ready\r\n"),
        Step::Expect("a1 LOGIN"),
        Step::Send("{tag} OK LOGIN completed\r\n"),
        Step::Expect("a2 CAPABILITY"),
        Step::Send(caps),
    ]
}

pub const PLAIN_CAPS: &str = "* CAPABILITY IMAP4rev1\r\n{tag} OK done\r\n";

/// Capabilities for the one-round-trip folder list (LIST-STATUS).
pub const STATUS_CAPS: &str =
    "* CAPABILITY IMAP4rev1 CONDSTORE LIST-EXTENDED LIST-STATUS\r\n{tag} OK done\r\n";

/// The command `Imap::list_status` sends, without its tag.
pub const LIST_STATUS: &str =
    r#"LIST "" "*" RETURN (STATUS (MESSAGES UIDNEXT UIDVALIDITY HIGHESTMODSEQ))"#;

pub const SELECT_OK: &str = concat!(
    "* 3 EXISTS\r\n",
    "* OK [UIDVALIDITY 7] ok\r\n",
    "* OK [UIDNEXT 12] ok\r\n",
    "{tag} OK [READ-WRITE] SELECT completed\r\n",
);

/// A SELECT reply like [`SELECT_OK`] with the given EXISTS and UIDNEXT.
pub fn select_reply(exists: u32, uidnext: u32) -> &'static str {
    leak(format!(
        "* {exists} EXISTS\r\n* OK [UIDVALIDITY 7] ok\r\n* OK [UIDNEXT {uidnext}] ok\r\n\
         {{tag}} OK [READ-WRITE] SELECT completed\r\n"
    ))
}

/// `line` is the expected LOGOUT command, tag included.
pub fn logout_steps(line: &'static str) -> Vec<Step> {
    vec![
        Step::Expect(line),
        Step::Send("* BYE bye\r\n{tag} OK LOGOUT completed\r\n"),
        Step::Close,
    ]
}

/// One FETCH reply line for `fetch_headers`, with INTERNALDATE
/// 2024-01-01 00:00:00 UTC.
pub fn header_fetch(seq: u32, uid: u32, flags: &str, header: &str) -> String {
    format!(
        "* {seq} FETCH (UID {uid} FLAGS ({flags}) INTERNALDATE \"01-Jan-2024 00:00:00 +0000\" \
         RFC822.SIZE 100 BODY[HEADER.FIELDS ({HEADER_FIELDS})] {{{}}}\r\n{header})\r\n",
        header.len()
    )
}

/// A tagged-OK reply to `fetch_bodies` with a small body for each UID.
pub fn body_fetch(uids: impl IntoIterator<Item = u32>) -> &'static str {
    let mut reply = String::new();
    for (i, uid) in uids.into_iter().enumerate() {
        let raw = format!("Subject: {uid}\r\n\r\nbody\r\n");
        reply += &format!(
            "* {} FETCH (UID {uid} BODY[] {{{}}}\r\n{raw})\r\n",
            i + 1,
            raw.len()
        );
    }
    leak(reply + "{tag} OK done\r\n")
}

/// `Step::Send` takes static text; this lets a test build a reply at run time.
pub fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// How [`FakeSmtp`] treats the message.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SmtpEnd {
    /// Accept it (250 after the data).
    Accept,
    /// Refuse every recipient with 550.
    RejectRcpt,
    /// Refuse every recipient for now with 450 (try again later).
    TempRejectRcpt,
    /// Read the whole message, then close without a reply.
    DropAfterData,
    /// Refuse the message after the data with 552 (too large).
    TooLarge,
    /// Refuse the sign-in (AUTH) with 535.
    RejectAuth,
    /// Refuse the sign-in for now with 454 (temporary failure).
    TempRejectAuth,
    /// Refuse the sender (MAIL FROM) with 553: not owned by the login.
    RejectSender,
}

/// A fake SMTP server for one client: answers 250 to everything except as
/// `end` says (EHLO offers AUTH PLAIN and LOGIN, and AUTH succeeds), and
/// records the first line, the RCPT lines and the DATA payload.
pub struct FakeSmtp {
    pub port: u16,
    handle: JoinHandle<(String, String, Vec<String>)>,
    auth: Arc<Mutex<Vec<String>>>,
}

impl FakeSmtp {
    pub fn start(end: SmtpEnd) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("binding the fake SMTP server");
        let port = listener.local_addr().expect("fake SMTP address").port();
        let auth = Arc::new(Mutex::new(Vec::new()));
        let recorded = auth.clone();
        let handle = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            s.write_all(b"220 fake\r\n").unwrap();
            let (mut data, mut in_data, mut line) = (String::new(), false, String::new());
            let mut rcpt = Vec::new();
            let mut first = None;
            loop {
                line.clear();
                if r.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                first.get_or_insert_with(|| line.clone());
                if in_data {
                    if line == ".\r\n" {
                        match end {
                            SmtpEnd::DropAfterData => {
                                s.shutdown(Shutdown::Both).ok();
                                break;
                            }
                            SmtpEnd::TooLarge => s.write_all(b"552 too big\r\n").unwrap(),
                            _ => s.write_all(b"250 ok\r\n").unwrap(),
                        }
                        in_data = false;
                    } else {
                        data.push_str(&line);
                    }
                    continue;
                }
                let up = line.to_ascii_uppercase();
                if up.starts_with("RCPT") {
                    rcpt.push(line.trim_end().to_string());
                }
                if up.starts_with("AUTH") {
                    recorded.lock().unwrap().push(line.trim_end().to_string());
                }
                // Write errors are ignored: the client may hang up first.
                let reply: &[u8] = if up.starts_with("EHLO") {
                    b"250-fake\r\n250 AUTH PLAIN LOGIN\r\n"
                } else if up.starts_with("AUTH") && end == SmtpEnd::RejectAuth {
                    b"535 5.7.8 bad credentials\r\n"
                } else if up.starts_with("AUTH") && end == SmtpEnd::TempRejectAuth {
                    b"454 4.7.0 Temporary authentication failure\r\n"
                } else if up.starts_with("AUTH") {
                    b"235 ok\r\n"
                } else if up.starts_with("DATA") {
                    in_data = true;
                    b"354 go\r\n"
                } else if up.starts_with("MAIL") && end == SmtpEnd::RejectSender {
                    b"553 5.7.1 Sender address rejected: not owned by user\r\n"
                } else if up.starts_with("RCPT") && end == SmtpEnd::RejectRcpt {
                    b"550 no such user\r\n"
                } else if up.starts_with("RCPT") && end == SmtpEnd::TempRejectRcpt {
                    b"450 mailbox busy, try later\r\n"
                } else if up.starts_with("QUIT") {
                    s.write_all(b"221 bye\r\n").ok();
                    break;
                } else {
                    b"250 ok\r\n"
                };
                s.write_all(reply).ok();
            }
            (first.unwrap_or_default(), data, rcpt)
        });
        Self { port, handle, auth }
    }

    /// Wait for the client to finish; returns its first line and the data.
    pub fn finish(self) -> (String, String) {
        let (first, data, _) = self.finish_with_rcpt();
        (first, data)
    }

    /// Wait for the client to finish; returns the `AUTH` lines it sent
    /// (`AUTH PLAIN <base64>`).
    pub fn finish_with_auth(self) -> Vec<String> {
        self.handle.join().expect("fake SMTP server panicked");
        self.auth.lock().unwrap().clone()
    }

    /// [`FakeSmtp::finish`], with the `RCPT` lines the client sent.
    pub fn finish_with_rcpt(self) -> (String, String, Vec<String>) {
        self.handle.join().expect("fake SMTP server panicked")
    }
}
