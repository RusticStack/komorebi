//! A stateful fake IMAP server for the capability profiles: a tiny mailbox
//! (folders of messages with UIDs and flags) that answers the commands the
//! workers send, with the extensions of a [`Profile`] and no others, and
//! records every line the client sent. Plain TCP; the secured paths have
//! their own fakes (`fake_tls`).
//!
//! It is as strict as the servers it stands for: a command whose extension
//! is not in the profile (`UID EXPUNGE` without UIDPLUS, `UID MOVE` without
//! MOVE) is answered `BAD`, and a bare `EXPUNGE` removes every message
//! flagged `\Deleted` in the folder, whoever flagged it.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

/// What the server can do.
#[derive(Clone, Copy, Debug)]
pub struct Profile {
    pub idle: bool,
    pub uidplus: bool,
    pub mv: bool,
    /// LIST carries `\Sent`, `\Trash` ... attributes.
    pub special_use: bool,
    /// The hierarchy delimiter LIST reports.
    pub delimiter: &'static str,
}

impl Profile {
    /// Nothing beyond IMAP4rev1.
    pub const BARE: Profile = Profile {
        idle: false,
        uidplus: false,
        mv: false,
        special_use: false,
        delimiter: "/",
    };

    /// Everything the workers can use, except CONDSTORE (this fake keeps no
    /// mod-sequences).
    pub const FULL: Profile = Profile {
        idle: true,
        uidplus: true,
        mv: true,
        special_use: true,
        delimiter: "/",
    };

    fn capabilities(&self) -> String {
        let mut caps = String::from("IMAP4rev1");
        for (on, name) in [
            (self.idle, "IDLE"),
            (self.uidplus, "UIDPLUS"),
            (self.mv, "MOVE"),
            (self.special_use, "SPECIAL-USE"),
        ] {
            if on {
                caps += " ";
                caps += name;
            }
        }
        caps
    }
}

#[derive(Clone, Debug)]
pub struct Msg {
    pub uid: u32,
    pub flags: BTreeSet<String>,
    pub raw: Vec<u8>,
}

impl Msg {
    pub fn has(&self, flag: &str) -> bool {
        self.flags.contains(flag)
    }
}

#[derive(Clone, Debug)]
pub struct MailBox {
    /// The raw name (modified UTF-7), as LIST reports and commands use it.
    pub name: String,
    /// The special-use attribute LIST shows with `special_use` (`\Sent`).
    pub attr: &'static str,
    pub noselect: bool,
    pub uidvalidity: u32,
    pub uidnext: u32,
    pub msgs: Vec<Msg>,
}

impl MailBox {
    pub fn new(name: &str, attr: &'static str) -> Self {
        Self {
            name: name.to_string(),
            attr,
            noselect: false,
            uidvalidity: 7,
            uidnext: 1,
            msgs: Vec::new(),
        }
    }

    /// A header-only listing entry that cannot be selected.
    pub fn header(name: &str) -> Self {
        Self {
            noselect: true,
            ..Self::new(name, "")
        }
    }

    pub fn add(&mut self, raw: Vec<u8>, flags: &[&str]) -> u32 {
        let uid = self.uidnext;
        self.uidnext += 1;
        self.msgs.push(Msg {
            uid,
            flags: flags.iter().map(|f| f.to_string()).collect(),
            raw,
        });
        uid
    }

    pub fn uids(&self) -> Vec<u32> {
        self.msgs.iter().map(|m| m.uid).collect()
    }

    pub fn msg(&self, uid: u32) -> Option<&Msg> {
        self.msgs.iter().find(|m| m.uid == uid)
    }

    pub fn msg_mut(&mut self, uid: u32) -> Option<&mut Msg> {
        self.msgs.iter_mut().find(|m| m.uid == uid)
    }
}

/// A message `n` of a corpus: a Message-ID and a subject.
pub fn sample(n: u32) -> Vec<u8> {
    format!(
        "From: Ann <ann@example.com>\r\nTo: me@example.com\r\nSubject: Message {n}\r\n\
         Message-ID: <m{n}@example.com>\r\nDate: Mon, 01 Jan 2024 00:00:00 +0000\r\n\r\n\
         Body {n}\r\n"
    )
    .into_bytes()
}

pub struct State {
    pub profile: Profile,
    pub boxes: Vec<MailBox>,
    /// Every line received, in order (a literal as one entry).
    pub log: Vec<String>,
    /// Bare `EXPUNGE` commands received.
    pub bare_expunges: u32,
    /// `UID COPY` / `UID MOVE` into a folder answered `NO` (the copy fails).
    pub refuse_copy_to: Option<String>,
    /// Sign-ins answered `NO`.
    pub refuse_login: bool,
    /// The greeting to send (`* OK ...` unless a test plays another).
    pub greeting: String,
}

impl State {
    pub fn mailbox(&self, name: &str) -> &MailBox {
        self.boxes
            .iter()
            .find(|b| b.name == name)
            .unwrap_or_else(|| panic!("no folder {name:?} on the fake server"))
    }

    pub fn mailbox_mut(&mut self, name: &str) -> &mut MailBox {
        self.boxes
            .iter_mut()
            .find(|b| b.name == name)
            .unwrap_or_else(|| panic!("no folder {name:?} on the fake server"))
    }

    /// The lines received that start with `verb` after the tag, e.g. `LOGIN`.
    pub fn sent(&self, verb: &str) -> Vec<&str> {
        self.log
            .iter()
            .filter(|l| {
                l.split_once(' ')
                    .is_some_and(|(_, rest)| rest.to_ascii_uppercase().starts_with(verb))
            })
            .map(String::as_str)
            .collect()
    }
}

pub struct FakeImapd {
    pub addr: SocketAddr,
    state: Arc<Mutex<State>>,
}

impl FakeImapd {
    pub fn start(profile: Profile, boxes: Vec<MailBox>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("binding the fake IMAP server");
        let addr = listener.local_addr().expect("fake IMAP address");
        let state = Arc::new(Mutex::new(State {
            profile,
            boxes,
            log: Vec::new(),
            bare_expunges: 0,
            refuse_copy_to: None,
            refuse_login: false,
            greeting: "* OK fake IMAP ready".into(),
        }));
        let shared = state.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let state = shared.clone();
                thread::spawn(move || {
                    serve(stream, &state).ok();
                });
            }
        });
        Self { addr, state }
    }

    pub fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The words of a command line: quoted strings unquoted, parentheses their
/// own words.
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut escaped, mut had) = (false, false, false);
    for c in line.chars() {
        if quoted {
            match (escaped, c) {
                (true, c) => {
                    cur.push(c);
                    escaped = false;
                }
                (false, '\\') => escaped = true,
                (false, '"') => quoted = false,
                (false, c) => cur.push(c),
            }
            continue;
        }
        match c {
            '"' => {
                quoted = true;
                had = true;
            }
            ' ' | '(' | ')' => {
                if !cur.is_empty() || had {
                    out.push(std::mem::take(&mut cur));
                    had = false;
                }
                if c != ' ' {
                    out.push(c.to_string());
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() || had {
        out.push(cur);
    }
    out
}

/// A UID set (`1:3,7,9:*`) over the UIDs present.
fn in_set(set: &str, uid: u32, max: u32) -> bool {
    set.split(',').any(|part| match part.split_once(':') {
        Some((a, b)) => {
            let num = |s: &str| {
                if s == "*" {
                    max
                } else {
                    s.parse().unwrap_or(0)
                }
            };
            let (a, b) = (num(a), num(b));
            (a.min(b)..=a.max(b)).contains(&uid)
        }
        None => {
            uid == if part == "*" {
                max
            } else {
                part.parse().unwrap_or(0)
            }
        }
    })
}

fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn flags_of(m: &Msg) -> String {
    m.flags.iter().cloned().collect::<Vec<_>>().join(" ")
}

fn serve(stream: TcpStream, state: &Mutex<State>) -> std::io::Result<()> {
    let mut out = stream.try_clone()?;
    let mut input = BufReader::new(stream);
    let lock = || state.lock().unwrap_or_else(|e| e.into_inner());
    let greeting = lock().greeting.clone();
    out.write_all(format!("{greeting}\r\n").as_bytes())?;
    let mut selected: Option<String> = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let line = line.trim_end_matches(['\r', '\n']).to_string();
        let Some((tag, command)) = line.split_once(' ') else {
            continue;
        };
        // The password stays out of the log's verbs; the line itself is kept
        // (tests read the sign-in back).
        lock().log.push(line.clone());
        let w = words(command);
        let upper: Vec<String> = w.iter().map(|x| x.to_ascii_uppercase()).collect();
        let verb = upper.first().map_or("", String::as_str);
        let (uid_cmd, args) = if verb == "UID" {
            (true, &w[2.min(w.len())..])
        } else {
            (false, &w[1.min(w.len())..])
        };
        let sub = if uid_cmd {
            upper.get(1).map_or("", String::as_str)
        } else {
            verb
        };
        let mut reply = String::new();
        let mut st = lock();
        let profile = st.profile;
        match sub {
            "CAPABILITY" => {
                reply += &format!(
                    "* CAPABILITY {}\r\n{tag} OK done\r\n",
                    profile.capabilities()
                );
            }
            "LOGIN" => {
                if st.refuse_login {
                    reply += &format!("{tag} NO [AUTHENTICATIONFAILED] bad credentials\r\n");
                } else {
                    reply += &format!("{tag} OK LOGIN completed\r\n");
                }
            }
            "NOOP" => reply += &format!("{tag} OK done\r\n"),
            "LOGOUT" => {
                drop(st);
                out.write_all(format!("* BYE bye\r\n{tag} OK LOGOUT completed\r\n").as_bytes())?;
                return Ok(());
            }
            "LIST" => {
                for b in &st.boxes {
                    let mut attrs = Vec::new();
                    if b.noselect {
                        attrs.push("\\Noselect");
                    } else {
                        attrs.push("\\HasNoChildren");
                    }
                    if profile.special_use && !b.attr.is_empty() {
                        attrs.push(b.attr);
                    }
                    reply += &format!(
                        "* LIST ({}) {} {}\r\n",
                        attrs.join(" "),
                        quoted(profile.delimiter),
                        quoted(&b.name)
                    );
                }
                reply += &format!("{tag} OK LIST completed\r\n");
            }
            "SELECT" | "EXAMINE" => {
                let name = args.first().cloned().unwrap_or_default();
                match st.boxes.iter().find(|b| b.name == name && !b.noselect) {
                    Some(b) => {
                        selected = Some(name);
                        reply += &format!(
                            "* FLAGS (\\Seen \\Answered \\Flagged \\Deleted \\Draft)\r\n\
                             * {} EXISTS\r\n* OK [UIDVALIDITY {}] ok\r\n* OK [UIDNEXT {}] ok\r\n\
                             {tag} OK [READ-WRITE] {sub} completed\r\n",
                            b.msgs.len(),
                            b.uidvalidity,
                            b.uidnext
                        );
                    }
                    None => {
                        selected = None;
                        reply += &format!("{tag} NO [NONEXISTENT] no such folder\r\n");
                    }
                }
            }
            "FETCH" => {
                let Some(b) = selected.as_deref().map(|n| st.mailbox(n)) else {
                    reply += &format!("{tag} BAD no folder selected\r\n");
                    out.write_all(reply.as_bytes())?;
                    continue;
                };
                let set = args.first().cloned().unwrap_or_default();
                let max = b.msgs.iter().map(|m| m.uid).max().unwrap_or(0);
                let query = command.to_ascii_uppercase();
                for (i, m) in b.msgs.iter().enumerate() {
                    if !in_set(&set, m.uid, max) {
                        continue;
                    }
                    let seq = i + 1;
                    if query.contains("HEADER.FIELDS") {
                        let end = m
                            .raw
                            .windows(4)
                            .position(|w| w == b"\r\n\r\n")
                            .map_or(m.raw.len(), |p| p + 4);
                        let header = String::from_utf8_lossy(&m.raw[..end]).into_owned();
                        reply +=
                            &super::fake::header_fetch(seq as u32, m.uid, &flags_of(m), &header);
                    } else if query.contains("BODY.PEEK[]") || query.contains("BODY[]") {
                        reply += &format!(
                            "* {seq} FETCH (UID {} BODY[] {{{}}}\r\n",
                            m.uid,
                            m.raw.len()
                        );
                        reply += &String::from_utf8_lossy(&m.raw);
                        reply += ")\r\n";
                    } else {
                        reply +=
                            &format!("* {seq} FETCH (UID {} FLAGS ({}))\r\n", m.uid, flags_of(m));
                    }
                }
                reply += &format!("{tag} OK FETCH completed\r\n");
            }
            "STORE" => {
                let Some(name) = selected.clone() else {
                    out.write_all(format!("{tag} BAD no folder selected\r\n").as_bytes())?;
                    continue;
                };
                let set = args.first().cloned().unwrap_or_default();
                let add = !args.get(1).is_some_and(|a| a.starts_with('-'));
                let flags: Vec<String> = args[2.min(args.len())..]
                    .iter()
                    .filter(|a| a.starts_with('\\'))
                    .cloned()
                    .collect();
                let b = st.mailbox_mut(&name);
                let max = b.msgs.iter().map(|m| m.uid).max().unwrap_or(0);
                for m in b.msgs.iter_mut().filter(|m| in_set(&set, m.uid, max)) {
                    for f in &flags {
                        if add {
                            m.flags.insert(f.clone());
                        } else {
                            m.flags.remove(f);
                        }
                    }
                }
                reply += &format!("{tag} OK STORE completed\r\n");
            }
            "COPY" | "MOVE" => {
                let Some(name) = selected.clone() else {
                    out.write_all(format!("{tag} BAD no folder selected\r\n").as_bytes())?;
                    continue;
                };
                if sub == "MOVE" && !profile.mv {
                    reply += &format!("{tag} BAD unknown command\r\n");
                    out.write_all(reply.as_bytes())?;
                    continue;
                }
                let set = args.first().cloned().unwrap_or_default();
                let dest = args.get(1).cloned().unwrap_or_default();
                if st.refuse_copy_to.as_deref() == Some(dest.as_str()) {
                    reply += &format!("{tag} NO [TRYCREATE] copy failed\r\n");
                } else if !st.boxes.iter().any(|b| b.name == dest) {
                    reply += &format!("{tag} NO [TRYCREATE] no such folder\r\n");
                } else {
                    let src = st.mailbox(&name);
                    let max = src.msgs.iter().map(|m| m.uid).max().unwrap_or(0);
                    let picked: Vec<Msg> = src
                        .msgs
                        .iter()
                        .filter(|m| in_set(&set, m.uid, max))
                        .cloned()
                        .collect();
                    for m in &picked {
                        let flags: Vec<&str> = m.flags.iter().map(String::as_str).collect();
                        st.mailbox_mut(&dest).add(m.raw.clone(), &flags);
                    }
                    if sub == "MOVE" {
                        let src = st.mailbox_mut(&name);
                        let before: Vec<u32> = src.uids();
                        src.msgs.retain(|m| !in_set(&set, m.uid, max));
                        for (i, uid) in before.iter().enumerate().rev() {
                            if in_set(&set, *uid, max) {
                                reply += &format!("* {} EXPUNGE\r\n", i + 1);
                            }
                        }
                    }
                    reply += &format!("{tag} OK {sub} completed\r\n");
                }
            }
            "EXPUNGE" => {
                let Some(name) = selected.clone() else {
                    out.write_all(format!("{tag} BAD no folder selected\r\n").as_bytes())?;
                    continue;
                };
                if uid_cmd && !profile.uidplus {
                    reply += &format!("{tag} BAD unknown command\r\n");
                    out.write_all(reply.as_bytes())?;
                    continue;
                }
                let set = if uid_cmd { args.first().cloned() } else { None };
                if !uid_cmd {
                    st.bare_expunges += 1;
                }
                let b = st.mailbox_mut(&name);
                let max = b.msgs.iter().map(|m| m.uid).max().unwrap_or(0);
                let mut gone = Vec::new();
                for (i, m) in b.msgs.iter().enumerate() {
                    let named = set.as_deref().is_none_or(|s| in_set(s, m.uid, max));
                    if m.has("\\Deleted") && named {
                        gone.push(i);
                    }
                }
                for (n, i) in gone.iter().enumerate() {
                    reply += &format!("* {} EXPUNGE\r\n", i + 1 - n);
                }
                for i in gone.into_iter().rev() {
                    b.msgs.remove(i);
                }
                reply += &format!("{tag} OK EXPUNGE completed\r\n");
            }
            "SEARCH" => {
                let Some(name) = selected.clone() else {
                    out.write_all(format!("{tag} BAD no folder selected\r\n").as_bytes())?;
                    continue;
                };
                let b = st.mailbox(&name);
                let query = command.to_ascii_uppercase();
                let wanted = query.contains("MESSAGE-ID").then(|| {
                    let at = upper.iter().position(|x| x == "MESSAGE-ID").unwrap_or(0);
                    w.get(at + 1)
                        .cloned()
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                });
                let undeleted = query.contains("UNDELETED");
                let found: Vec<String> = b
                    .msgs
                    .iter()
                    .filter(|m| !(undeleted && m.has("\\Deleted")))
                    .filter(|m| {
                        wanted.as_ref().is_none_or(|id| {
                            String::from_utf8_lossy(&m.raw)
                                .to_ascii_lowercase()
                                .contains(&format!("message-id: {id}"))
                        })
                    })
                    .map(|m| m.uid.to_string())
                    .collect();
                let hits = if found.is_empty() {
                    String::new()
                } else {
                    format!(" {}", found.join(" "))
                };
                reply += &format!("* SEARCH{hits}\r\n{tag} OK SEARCH completed\r\n");
            }
            "APPEND" => {
                let name = args.first().cloned().unwrap_or_default();
                let n: usize = command
                    .rsplit_once('{')
                    .and_then(|(_, n)| n.strip_suffix('}')?.parse().ok())
                    .unwrap_or(0);
                if !st.boxes.iter().any(|b| b.name == name) {
                    drop(st);
                    out.write_all(format!("{tag} NO [TRYCREATE] no such folder\r\n").as_bytes())?;
                    continue;
                }
                drop(st);
                out.write_all(b"+ go ahead\r\n")?;
                let mut data = vec![0; n + 2];
                input.read_exact(&mut data)?;
                data.truncate(n);
                let flags: Vec<&str> = args
                    .iter()
                    .filter(|a| a.starts_with('\\'))
                    .map(String::as_str)
                    .collect();
                let mut st = lock();
                st.log.push(String::from_utf8_lossy(&data).into_owned());
                let b = st.mailbox_mut(&name);
                let uid = b.add(data, &flags);
                let uv = b.uidvalidity;
                let code = if profile.uidplus {
                    format!("[APPENDUID {uv} {uid}] ")
                } else {
                    String::new()
                };
                out.write_all(format!("{tag} OK {code}APPEND completed\r\n").as_bytes())?;
                continue;
            }
            _ => reply += &format!("{tag} BAD unknown command\r\n"),
        }
        drop(st);
        out.write_all(reply.as_bytes())?;
    }
}
