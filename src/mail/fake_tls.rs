//! A TLS-capable fake IMAP or SMTP server for tests: the throwaway identity
//! in `tests/fixtures/tls` (or the second one, for a "swapped" certificate)
//! in front of a tiny scripted server, implicit TLS or STARTTLS. It records
//! every line the client sends, plain or secure, so a test can assert that
//! no credentials arrived.

use native_tls::{Certificate, Identity, TlsAcceptor, TlsStream};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub(crate) const CERT: &[u8] = include_bytes!("../../tests/fixtures/tls/test-cert.pem");
pub(crate) const KEY: &[u8] = include_bytes!("../../tests/fixtures/tls/test-key.pem");
pub(crate) const OTHER_CERT: &[u8] = include_bytes!("../../tests/fixtures/tls/other-cert.pem");
pub(crate) const OTHER_KEY: &[u8] = include_bytes!("../../tests/fixtures/tls/other-key.pem");

pub(crate) fn der_of(pem: &[u8]) -> Vec<u8> {
    Certificate::from_pem(pem).unwrap().to_der().unwrap()
}

pub(crate) fn acceptor(cert: &'static [u8], key: &'static [u8]) -> Arc<TlsAcceptor> {
    // One acceptor per identity (Windows keeps its key in a container).
    type Cache = Mutex<Vec<(usize, Arc<TlsAcceptor>)>>;
    static ACCEPTORS: OnceLock<Cache> = OnceLock::new();
    let mut all = ACCEPTORS.get_or_init(Default::default).lock().unwrap();
    let id = cert.as_ptr() as usize;
    if let Some((_, a)) = all.iter().find(|(i, _)| *i == id) {
        return a.clone();
    }
    let a = Arc::new(TlsAcceptor::new(Identity::from_pkcs8(cert, key).unwrap()).unwrap());
    all.push((id, a.clone()));
    a
}

/// An IMAP or SMTP fake behind TLS: implicit, or STARTTLS (plain
/// greeting first). Records every line the client sends, plain or
/// secure, and answers just enough for a sign-in.
pub(crate) struct TlsFake {
    pub(crate) addr: SocketAddr,
    handle: JoinHandle<Vec<String>>,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Proto {
    Imap,
    Smtp,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Mode {
    Implicit,
    StartTls,
    /// STARTTLS is not offered and AUTH/LOGIN is offered in the clear.
    NoStartTls,
    /// IMAP: STARTTLS is listed but answered `NO`; the clear sign-in is on
    /// offer.
    StartTlsRefused,
}

impl TlsFake {
    pub(crate) fn start(proto: Proto, mode: Mode, cert: &'static [u8], key: &'static [u8]) -> Self {
        Self::start_with(proto, mode, cert, key, false)
    }

    /// The same, answering IMAP LOGIN with `NO [AUTHENTICATIONFAILED]`
    /// when `reject_login` (a wrong password).
    pub(crate) fn start_with(
        proto: Proto,
        mode: Mode,
        cert: &'static [u8],
        key: &'static [u8],
        reject_login: bool,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let acceptor = acceptor(cert, key);
        let handle = thread::spawn(move || {
            let mut seen = Vec::new();
            let Ok((tcp, _)) = listener.accept() else {
                return seen;
            };
            tcp.set_read_timeout(Some(Duration::from_secs(10))).ok();
            let greeting: &[u8] = match proto {
                Proto::Imap => b"* OK fake ready\r\n",
                Proto::Smtp => b"220 fake ready\r\n",
            };
            let tcp = if mode == Mode::Implicit {
                match acceptor.accept(tcp) {
                    Ok(t) => Secure::Tls(Box::new(t)),
                    Err(_) => return seen,
                }
            } else {
                Secure::Plain(tcp)
            };
            let mut io = tcp;
            io.write_all(greeting).ok();
            let mut upgraded = mode == Mode::Implicit;
            while let Some(line) = io.read_line() {
                seen.push(line.clone());
                let up = line.to_ascii_uppercase();
                let reply: String = match proto {
                    Proto::Imap => {
                        let tag = line.split(' ').next().unwrap_or("a").to_string();
                        if up.contains("STARTTLS") && !upgraded {
                            if matches!(mode, Mode::NoStartTls | Mode::StartTlsRefused) {
                                format!("{tag} NO no TLS here\r\n")
                            } else {
                                io.write_all(format!("{tag} OK begin TLS\r\n").as_bytes())
                                    .ok();
                                match io.upgrade(&acceptor) {
                                    Some(()) => {
                                        upgraded = true;
                                        continue;
                                    }
                                    None => break,
                                }
                            }
                        } else if up.contains(" LOGIN") && reject_login {
                            format!("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n")
                        } else if up.contains(" LOGIN") {
                            format!("{tag} OK LOGIN completed\r\n")
                        } else if up.contains(" CAPABILITY") {
                            // Before TLS: STARTTLS on offer and LOGIN
                            // disabled (as real servers say it); after: PLAIN.
                            let caps = match (upgraded, mode) {
                                (true, _) | (_, Mode::Implicit) => "IMAP4rev1 AUTH=PLAIN",
                                (false, Mode::NoStartTls) => "IMAP4rev1",
                                (false, _) => "IMAP4rev1 STARTTLS LOGINDISABLED",
                            };
                            format!("* CAPABILITY {caps}\r\n{tag} OK done\r\n")
                        } else if up.contains(" LOGOUT") {
                            io.write_all(format!("* BYE\r\n{tag} OK bye\r\n").as_bytes())
                                .ok();
                            break;
                        } else {
                            format!("{tag} OK done\r\n")
                        }
                    }
                    Proto::Smtp => {
                        if up.starts_with("EHLO") {
                            let mut r = String::from("250-fake\r\n");
                            if !matches!(mode, Mode::NoStartTls) && !upgraded {
                                r += "250-STARTTLS\r\n";
                            }
                            r + "250 AUTH PLAIN LOGIN\r\n"
                        } else if up.starts_with("STARTTLS") {
                            io.write_all(b"220 go\r\n").ok();
                            match io.upgrade(&acceptor) {
                                Some(()) => {
                                    upgraded = true;
                                    continue;
                                }
                                None => break,
                            }
                        } else if up.starts_with("AUTH") {
                            "235 ok\r\n".to_string()
                        } else if up.starts_with("QUIT") {
                            io.write_all(b"221 bye\r\n").ok();
                            break;
                        } else {
                            "250 ok\r\n".to_string()
                        }
                    }
                };
                io.write_all(reply.as_bytes()).ok();
            }
            seen
        });
        Self { addr, handle }
    }

    pub(crate) fn finish(self) -> Vec<String> {
        self.handle.join().unwrap()
    }
}

enum Secure {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
    Gone,
}

impl Secure {
    fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
        match self {
            Secure::Plain(s) => s.write_all(data),
            Secure::Tls(s) => s.write_all(data),
            Secure::Gone => Ok(()),
        }
    }

    /// One line without its CRLF, reading byte by byte so no byte
    /// is taken from the stream before an upgrade.
    fn read_line(&mut self) -> Option<String> {
        let mut line = Vec::new();
        let mut b = [0u8; 1];
        loop {
            let n = match self {
                Secure::Plain(s) => s.read(&mut b),
                Secure::Tls(s) => s.read(&mut b),
                Secure::Gone => return None,
            }
            .ok()?;
            if n == 0 {
                return None;
            }
            if b[0] == b'\n' {
                return Some(
                    String::from_utf8_lossy(&line)
                        .trim_end_matches('\r')
                        .to_string(),
                );
            }
            line.push(b[0]);
        }
    }

    fn upgrade(&mut self, acceptor: &TlsAcceptor) -> Option<()> {
        let Secure::Plain(tcp) = std::mem::replace(self, Secure::Gone) else {
            return None;
        };
        *self = Secure::Tls(Box::new(acceptor.accept(tcp).ok()?));
        Some(())
    }
}
