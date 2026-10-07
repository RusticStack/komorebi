//! Thin IMAP session wrapper (blocking, used from worker threads only).

use super::mime::{HEADER_FIELDS, parse_header};
use crate::db::{self, NewHeader};
use crate::providers::{Security, Server};
use crate::role::Role;
use anyhow::{Context, Result, anyhow};
use imap::types::{Flag, NameAttribute};
use native_tls::{TlsConnector, TlsStream};
use std::collections::HashMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

type Session = imap::Session<TlsStream<TcpStream>>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const IO_TIMEOUT: Duration = Duration::from_secs(90);

pub struct Imap {
    session: Session,
    selected: Option<String>,
    has_move: bool,
    has_uidplus: bool,
}

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
}

impl Imap {
    pub fn connect(server: &Server, username: &str, password: &str) -> Result<Self> {
        let addr = (server.host.as_str(), server.port)
            .to_socket_addrs()
            .with_context(|| format!("resolving {}", server.host))?
            .next()
            .ok_or_else(|| anyhow!("{} has no address", server.host))?;
        let tcp = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
            .with_context(|| format!("connecting to {}:{}", server.host, server.port))?;
        tcp.set_read_timeout(Some(IO_TIMEOUT))?;
        tcp.set_write_timeout(Some(IO_TIMEOUT))?;
        let tls = TlsConnector::new().context("initializing TLS")?;
        let client = match server.security {
            Security::Tls => {
                let stream = tls
                    .connect(&server.host, tcp)
                    .map_err(|e| anyhow!("TLS handshake with {} failed: {e}", server.host))?;
                let mut client = imap::Client::new(stream);
                client.read_greeting().context("reading IMAP greeting")?;
                client
            }
            Security::StartTls => {
                let mut client = imap::Client::new(tcp);
                client.read_greeting().context("reading IMAP greeting")?;
                client
                    .secure(&server.host, &tls)
                    .context("IMAP STARTTLS failed")?
            }
        };
        let mut session = client
            .login(username, password)
            .map_err(|(e, _)| anyhow!("sign-in failed: {e}"))?;
        let (has_move, has_uidplus) = match session.capabilities() {
            Ok(caps) => (caps.has_str("MOVE"), caps.has_str("UIDPLUS")),
            Err(_) => (false, false),
        };
        Ok(Self {
            session,
            selected: None,
            has_move,
            has_uidplus,
        })
    }

    pub fn noop(&mut self) -> Result<()> {
        Ok(self.session.noop()?)
    }

    pub fn list_folders(&mut self) -> Result<Vec<RemoteFolder>> {
        let names = self.session.list(Some(""), Some("*"))?;
        let mut out = Vec::new();
        for n in names.iter() {
            let attrs = n.attributes();
            if attrs.iter().any(|a| matches!(a, NameAttribute::NoSelect)) {
                continue;
            }
            let special = attrs.iter().find_map(|a| match a {
                NameAttribute::Custom(v) => Role::from_attr(v),
                _ => None,
            });
            out.push(remote_folder(n.name(), n.delimiter(), special));
        }
        Ok(out)
    }

    pub fn select(&mut self, folder: &str) -> Result<Selected> {
        let mb = self
            .session
            .select(folder)
            .with_context(|| format!("opening folder {folder}"))?;
        self.selected = Some(folder.to_string());
        Ok(Selected {
            uidvalidity: mb.uid_validity.unwrap_or(0),
            exists: mb.exists,
        })
    }

    fn ensure_selected(&mut self, folder: &str) -> Result<()> {
        if self.selected.as_deref() != Some(folder) {
            self.select(folder)?;
        }
        Ok(())
    }

    /// UID → flag bits of every message in the selected folder, in one
    /// round trip (replaces SEARCH ALL/UNSEEN/FLAGGED/ANSWERED).
    pub fn uid_flags(&mut self) -> Result<HashMap<u32, u8>> {
        let fetches = self.session.uid_fetch("1:*", "(UID FLAGS)")?;
        Ok(fetches
            .iter()
            .filter_map(|f| Some((f.uid?, flag_bits(f.flags()))))
            .collect())
    }

    pub fn fetch_headers(&mut self, uids: &[u32]) -> Result<Vec<NewHeader>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let fetches = self.session.uid_fetch(
            uid_set(uids),
            format!("(UID FLAGS RFC822.SIZE BODY.PEEK[HEADER.FIELDS ({HEADER_FIELDS})])"),
        )?;
        Ok(fetches
            .iter()
            .filter_map(|f| {
                let uid = f.uid?;
                let mut h = parse_header(f.header().unwrap_or_default());
                h.uid = uid;
                h.size = f.size.unwrap_or(0) as i64;
                let bits = flag_bits(f.flags());
                h.seen = bits & db::SEEN != 0;
                h.flagged = bits & db::FLAGGED != 0;
                h.answered = bits & db::ANSWERED != 0;
                Some(h)
            })
            .collect())
    }

    /// Full RFC 822 bytes of messages, without setting \Seen.
    pub fn fetch_bodies(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<(u32, Vec<u8>)>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        self.ensure_selected(folder)?;
        let fetches = self.session.uid_fetch(uid_set(uids), "(UID BODY.PEEK[])")?;
        Ok(fetches
            .iter()
            .filter_map(|f| Some((f.uid?, f.body()?.to_vec())))
            .collect())
    }

    pub fn set_flag(&mut self, folder: &str, uid: u32, flag: &str, on: bool) -> Result<()> {
        self.ensure_selected(folder)?;
        let op = if on { "+FLAGS.SILENT" } else { "-FLAGS.SILENT" };
        self.session
            .uid_store(uid.to_string(), format!("{op} ({flag})"))?;
        Ok(())
    }

    /// Move a message to another folder (MOVE, or COPY + delete).
    pub fn move_message(&mut self, folder: &str, uid: u32, dest: &str) -> Result<()> {
        self.ensure_selected(folder)?;
        if self.has_move {
            self.session.uid_mv(uid.to_string(), dest)?;
        } else {
            self.session.uid_copy(uid.to_string(), dest)?;
            self.expunge(uid)?;
        }
        Ok(())
    }

    /// Permanently remove a message from the selected folder.
    pub fn delete_message(&mut self, folder: &str, uid: u32) -> Result<()> {
        self.ensure_selected(folder)?;
        self.expunge(uid)
    }

    fn expunge(&mut self, uid: u32) -> Result<()> {
        self.session
            .uid_store(uid.to_string(), "+FLAGS.SILENT (\\Deleted)")?;
        if self.has_uidplus {
            self.session.uid_expunge(uid.to_string())?;
        } else {
            // Without UIDPLUS, EXPUNGE also removes anything another client
            // marked \Deleted in this folder (standard IMAP behavior).
            self.session.expunge()?;
        }
        Ok(())
    }

    pub fn append(&mut self, folder: &str, raw: &[u8]) -> Result<()> {
        self.session
            .append_with_flags(folder, raw, &[Flag::Seen])
            .with_context(|| format!("saving a copy to {folder}"))
    }

    pub fn logout(mut self) {
        self.session.logout().ok();
    }
}

pub fn flag_bits(flags: &[Flag]) -> u8 {
    flags.iter().fold(0, |bits, f| {
        bits | match f {
            Flag::Seen => db::SEEN,
            Flag::Flagged => db::FLAGGED,
            Flag::Answered => db::ANSWERED,
            _ => 0,
        }
    })
}

fn uid_set(uids: &[u32]) -> String {
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    let mut out = String::new();
    let mut i = 0;
    while i < sorted.len() {
        let start = sorted[i];
        let mut end = start;
        while i + 1 < sorted.len() && sorted[i + 1] == end + 1 {
            i += 1;
            end = sorted[i];
        }
        if !out.is_empty() {
            out.push(',');
        }
        if start == end {
            out.push_str(&start.to_string());
        } else {
            out.push_str(&format!("{start}:{end}"));
        }
        i += 1;
    }
    out
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
    let role = match Role::from_name(name, &display) {
        Some(Role::Inbox) => Role::Inbox,
        by_name => special.or(by_name).unwrap_or_default(),
    };
    RemoteFolder {
        name: name.to_string(),
        display: if role == Role::Inbox {
            "Inbox".to_string()
        } else {
            display
        },
        depth: if role == Role::Other { depth } else { 0 },
        role,
    }
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
        } else if let Some(units) = decode_mbase64(encoded) {
            out.push_str(&String::from_utf16_lossy(&units));
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
    Some(
        bytes
            .chunks_exact(2)
            .map(|p| u16::from_be_bytes([p[0], p[1]]))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_flags() {
        assert_eq!(
            flag_bits(&[Flag::Seen, Flag::Answered, Flag::Deleted]),
            db::SEEN | db::ANSWERED
        );
        assert_eq!(flag_bits(&[]), 0);
    }

    #[test]
    fn compresses_uid_sets() {
        assert_eq!(uid_set(&[5, 1, 2, 3, 9, 10]), "1:3,5,9:10");
        assert_eq!(uid_set(&[7]), "7");
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
}
