use std::{
    error::Error,
    fmt,
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};

use base64::{engine::general_purpose, Engine as _};
use native_tls::{TlsConnector, TlsStream};

use crate::token_lifecycle::SecretString;

const CONNECTION_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_PROTOCOL_LINE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailTransportSecurity {
    Tls,
    StartTls,
    #[cfg(test)]
    PlaintextForTests,
}

impl MailTransportSecurity {
    pub fn from_schema(value: &str) -> Result<Self, AccountConnectionTestError> {
        match value {
            "tls" => Ok(Self::Tls),
            "starttls" => Ok(Self::StartTls),
            "none" => Err(AccountConnectionTestError::PlaintextDenied),
            _ => Err(AccountConnectionTestError::InvalidSecurity),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GenericAccountConnectionConfig {
    pub username: String,
    pub secret: SecretString,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: MailTransportSecurity,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: MailTransportSecurity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountConnectionTestError {
    InvalidSecurity,
    PlaintextDenied,
    NetworkUnavailable { protocol: &'static str },
    TlsHandshakeFailed { protocol: &'static str },
    StartTlsUnavailable { protocol: &'static str },
    ProtocolRejected { protocol: &'static str },
    AuthenticationRejected { protocol: &'static str },
}

impl fmt::Display for AccountConnectionTestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSecurity => formatter.write_str("mail transport security is unsupported"),
            Self::PlaintextDenied => {
                formatter.write_str("mail transport security must use TLS or STARTTLS")
            }
            Self::NetworkUnavailable { protocol } => {
                write!(formatter, "{protocol} connection could not be established")
            }
            Self::TlsHandshakeFailed { protocol } => {
                write!(formatter, "{protocol} TLS negotiation failed")
            }
            Self::StartTlsUnavailable { protocol } => {
                write!(formatter, "{protocol} STARTTLS was not accepted")
            }
            Self::ProtocolRejected { protocol } => {
                write!(formatter, "{protocol} server response was not accepted")
            }
            Self::AuthenticationRejected { protocol } => {
                write!(formatter, "{protocol} authentication failed")
            }
        }
    }
}

impl Error for AccountConnectionTestError {}

pub trait GenericAccountConnectionTester {
    fn test_generic_account(
        &mut self,
        config: &GenericAccountConnectionConfig,
    ) -> Result<(), AccountConnectionTestError>;
}

#[derive(Debug, Clone)]
pub struct LiveGenericAccountConnectionTester {
    timeout: Duration,
}

impl Default for LiveGenericAccountConnectionTester {
    fn default() -> Self {
        Self {
            timeout: CONNECTION_TIMEOUT,
        }
    }
}

impl GenericAccountConnectionTester for LiveGenericAccountConnectionTester {
    fn test_generic_account(
        &mut self,
        config: &GenericAccountConnectionConfig,
    ) -> Result<(), AccountConnectionTestError> {
        test_imap_login(config, self.timeout)?;
        test_smtp_auth(config, self.timeout)
    }
}

trait ProtocolStream: Read + Write {}

impl<T: Read + Write> ProtocolStream for T {}

fn connect_tcp(
    host: &str,
    port: u16,
    timeout: Duration,
    protocol: &'static str,
) -> Result<TcpStream, AccountConnectionTestError> {
    let mut addrs = (host, port)
        .to_socket_addrs()
        .map_err(|_| AccountConnectionTestError::NetworkUnavailable { protocol })?;
    for addr in addrs.by_ref() {
        let Ok(stream) = TcpStream::connect_timeout(&addr, timeout) else {
            continue;
        };
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|_| AccountConnectionTestError::NetworkUnavailable { protocol })?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|_| AccountConnectionTestError::NetworkUnavailable { protocol })?;

        return Ok(stream);
    }

    Err(AccountConnectionTestError::NetworkUnavailable { protocol })
}

fn build_tls_connector(protocol: &'static str) -> Result<TlsConnector, AccountConnectionTestError> {
    TlsConnector::builder()
        .build()
        .map_err(|_| AccountConnectionTestError::TlsHandshakeFailed { protocol })
}

fn wrap_tls(
    connector: &TlsConnector,
    host: &str,
    stream: TcpStream,
    protocol: &'static str,
) -> Result<TlsStream<TcpStream>, AccountConnectionTestError> {
    connector
        .connect(host, stream)
        .map_err(|_| AccountConnectionTestError::TlsHandshakeFailed { protocol })
}

fn test_imap_login(
    config: &GenericAccountConnectionConfig,
    timeout: Duration,
) -> Result<(), AccountConnectionTestError> {
    let protocol = "IMAP";
    let connector = build_tls_connector(protocol)?;
    let stream = connect_tcp(&config.imap_host, config.imap_port, timeout, protocol)?;
    let mut stream: Box<dyn ProtocolStream> = match config.imap_security {
        MailTransportSecurity::Tls => {
            Box::new(wrap_tls(&connector, &config.imap_host, stream, protocol)?)
        }
        MailTransportSecurity::StartTls => {
            let mut stream = stream;
            read_imap_greeting(&mut stream)?;
            write_protocol_line(&mut stream, "A001 STARTTLS", protocol)?;
            let lines = read_imap_until_tag(&mut stream, "A001")?;
            require_imap_ok(
                &lines,
                protocol,
                AccountConnectionTestError::StartTlsUnavailable { protocol },
            )?;
            Box::new(wrap_tls(&connector, &config.imap_host, stream, protocol)?)
        }
        #[cfg(test)]
        MailTransportSecurity::PlaintextForTests => Box::new(stream),
    };

    if config.imap_security != MailTransportSecurity::StartTls {
        read_imap_greeting(stream.as_mut())?;
    }

    let login_command = format!(
        "A002 LOGIN {} {}",
        quote_imap_string(&config.username),
        quote_imap_string(config.secret.expose_secret())
    );
    write_protocol_line(stream.as_mut(), &login_command, protocol)?;
    let login_lines = read_imap_until_tag(stream.as_mut(), "A002")?;
    require_imap_ok(
        &login_lines,
        protocol,
        AccountConnectionTestError::AuthenticationRejected { protocol },
    )?;

    let _ = write_protocol_line(stream.as_mut(), "A003 LOGOUT", protocol);
    let _ = read_imap_until_tag(stream.as_mut(), "A003");
    Ok(())
}

fn read_imap_greeting(stream: &mut dyn ProtocolStream) -> Result<(), AccountConnectionTestError> {
    let line = read_protocol_line(stream, "IMAP")?;
    if line.starts_with("* OK") || line.starts_with("* PREAUTH") {
        Ok(())
    } else {
        Err(AccountConnectionTestError::ProtocolRejected { protocol: "IMAP" })
    }
}

fn read_imap_until_tag(
    stream: &mut dyn ProtocolStream,
    tag: &str,
) -> Result<Vec<String>, AccountConnectionTestError> {
    let mut lines = Vec::new();
    let tagged_prefix = format!("{tag} ");
    loop {
        let line = read_protocol_line(stream, "IMAP")?;
        let is_tagged = line.starts_with(&tagged_prefix);
        lines.push(line);
        if is_tagged {
            return Ok(lines);
        }
    }
}

fn require_imap_ok(
    lines: &[String],
    protocol: &'static str,
    error: AccountConnectionTestError,
) -> Result<(), AccountConnectionTestError> {
    let Some(tagged) = lines.last() else {
        return Err(AccountConnectionTestError::ProtocolRejected { protocol });
    };
    if tagged.split_whitespace().nth(1) == Some("OK") {
        Ok(())
    } else {
        Err(error)
    }
}

fn quote_imap_string(value: &str) -> String {
    let mut quoted = String::from("\"");
    for character in value.chars() {
        if matches!(character, '"' | '\\') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

fn test_smtp_auth(
    config: &GenericAccountConnectionConfig,
    timeout: Duration,
) -> Result<(), AccountConnectionTestError> {
    let protocol = "SMTP";
    let connector = build_tls_connector(protocol)?;
    let stream = connect_tcp(&config.smtp_host, config.smtp_port, timeout, protocol)?;
    let mut stream: Box<dyn ProtocolStream> = match config.smtp_security {
        MailTransportSecurity::Tls => {
            Box::new(wrap_tls(&connector, &config.smtp_host, stream, protocol)?)
        }
        MailTransportSecurity::StartTls => {
            let mut stream = stream;
            require_smtp_code(&read_smtp_response(&mut stream)?, 220, protocol)?;
            let ehlo = smtp_ehlo(&mut stream)?;
            if !smtp_lines_contain_capability(&ehlo.lines, "STARTTLS") {
                return Err(AccountConnectionTestError::StartTlsUnavailable { protocol });
            }
            write_protocol_line(&mut stream, "STARTTLS", protocol)?;
            require_smtp_code(&read_smtp_response(&mut stream)?, 220, protocol)?;
            Box::new(wrap_tls(&connector, &config.smtp_host, stream, protocol)?)
        }
        #[cfg(test)]
        MailTransportSecurity::PlaintextForTests => Box::new(stream),
    };

    if config.smtp_security != MailTransportSecurity::StartTls {
        require_smtp_code(&read_smtp_response(stream.as_mut())?, 220, protocol)?;
    }

    let ehlo = smtp_ehlo(stream.as_mut())?;
    let supports_plain = smtp_auth_supports(&ehlo.lines, "PLAIN");
    let supports_login = smtp_auth_supports(&ehlo.lines, "LOGIN");

    if supports_plain {
        smtp_auth_plain(stream.as_mut(), &config.username, &config.secret)?;
    } else if supports_login {
        smtp_auth_login(stream.as_mut(), &config.username, &config.secret)?;
    } else {
        return Err(AccountConnectionTestError::ProtocolRejected { protocol });
    }

    let _ = write_protocol_line(stream.as_mut(), "QUIT", protocol);
    let _ = read_smtp_response(stream.as_mut());
    Ok(())
}

fn smtp_ehlo(stream: &mut dyn ProtocolStream) -> Result<SmtpResponse, AccountConnectionTestError> {
    write_protocol_line(stream, "EHLO komorebi.local", "SMTP")?;
    require_smtp_code(&read_smtp_response(stream)?, 250, "SMTP")
}

fn smtp_auth_plain(
    stream: &mut dyn ProtocolStream,
    username: &str,
    secret: &SecretString,
) -> Result<(), AccountConnectionTestError> {
    let payload = format!("\0{username}\0{}", secret.expose_secret());
    let encoded = general_purpose::STANDARD.encode(payload.as_bytes());
    write_protocol_line(stream, &format!("AUTH PLAIN {encoded}"), "SMTP")?;
    require_smtp_code(&read_smtp_response(stream)?, 235, "SMTP")
        .map_err(|_| AccountConnectionTestError::AuthenticationRejected { protocol: "SMTP" })
        .map(|_| ())
}

fn smtp_auth_login(
    stream: &mut dyn ProtocolStream,
    username: &str,
    secret: &SecretString,
) -> Result<(), AccountConnectionTestError> {
    write_protocol_line(stream, "AUTH LOGIN", "SMTP")?;
    require_smtp_code(&read_smtp_response(stream)?, 334, "SMTP")
        .map_err(|_| AccountConnectionTestError::AuthenticationRejected { protocol: "SMTP" })?;
    write_protocol_line(
        stream,
        &general_purpose::STANDARD.encode(username.as_bytes()),
        "SMTP",
    )?;
    require_smtp_code(&read_smtp_response(stream)?, 334, "SMTP")
        .map_err(|_| AccountConnectionTestError::AuthenticationRejected { protocol: "SMTP" })?;
    write_protocol_line(
        stream,
        &general_purpose::STANDARD.encode(secret.expose_secret().as_bytes()),
        "SMTP",
    )?;
    require_smtp_code(&read_smtp_response(stream)?, 235, "SMTP")
        .map_err(|_| AccountConnectionTestError::AuthenticationRejected { protocol: "SMTP" })
        .map(|_| ())
}

#[derive(Debug, Clone)]
struct SmtpResponse {
    code: u16,
    lines: Vec<String>,
}

fn read_smtp_response(
    stream: &mut dyn ProtocolStream,
) -> Result<SmtpResponse, AccountConnectionTestError> {
    let mut lines = Vec::new();
    let mut code = None;
    loop {
        let line = read_protocol_line(stream, "SMTP")?;
        if line.len() < 3 || !line.as_bytes()[0..3].iter().all(u8::is_ascii_digit) {
            return Err(AccountConnectionTestError::ProtocolRejected { protocol: "SMTP" });
        }
        let line_code = line[0..3]
            .parse::<u16>()
            .map_err(|_| AccountConnectionTestError::ProtocolRejected { protocol: "SMTP" })?;
        let is_last = line.as_bytes().get(3).copied() != Some(b'-');
        code.get_or_insert(line_code);
        lines.push(line);
        if is_last {
            return Ok(SmtpResponse {
                code: code.unwrap_or(line_code),
                lines,
            });
        }
    }
}

fn require_smtp_code(
    response: &SmtpResponse,
    expected: u16,
    protocol: &'static str,
) -> Result<SmtpResponse, AccountConnectionTestError> {
    if response.code == expected {
        Ok(response.clone())
    } else if response.code == 535 || response.code == 534 {
        Err(AccountConnectionTestError::AuthenticationRejected { protocol })
    } else {
        Err(AccountConnectionTestError::ProtocolRejected { protocol })
    }
}

fn smtp_lines_contain_capability(lines: &[String], capability: &str) -> bool {
    lines.iter().any(|line| {
        line.get(4..)
            .map(|value| {
                value
                    .split_whitespace()
                    .any(|token| token.eq_ignore_ascii_case(capability))
            })
            .unwrap_or(false)
    })
}

fn smtp_auth_supports(lines: &[String], mechanism: &str) -> bool {
    lines.iter().any(|line| {
        let Some(rest) = line.get(4..) else {
            return false;
        };
        let mut parts = rest.split_whitespace();
        matches!(parts.next(), Some(value) if value.eq_ignore_ascii_case("AUTH"))
            && parts.any(|token| token.eq_ignore_ascii_case(mechanism))
    })
}

fn read_protocol_line(
    stream: &mut dyn ProtocolStream,
    protocol: &'static str,
) -> Result<String, AccountConnectionTestError> {
    let mut bytes = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let read = stream
            .read(&mut byte)
            .map_err(|_| AccountConnectionTestError::NetworkUnavailable { protocol })?;
        if read == 0 {
            return Err(AccountConnectionTestError::NetworkUnavailable { protocol });
        }
        bytes.push(byte[0]);
        if bytes.len() > MAX_PROTOCOL_LINE_BYTES {
            return Err(AccountConnectionTestError::ProtocolRejected { protocol });
        }
        if byte[0] == b'\n' {
            break;
        }
    }

    let line = String::from_utf8_lossy(&bytes)
        .trim_end_matches(['\r', '\n'])
        .to_string();
    Ok(line)
}

fn write_protocol_line(
    stream: &mut dyn ProtocolStream,
    line: &str,
    protocol: &'static str,
) -> Result<(), AccountConnectionTestError> {
    stream
        .write_all(line.as_bytes())
        .and_then(|_| stream.write_all(b"\r\n"))
        .and_then(|_| stream.flush())
        .map_err(|_| AccountConnectionTestError::NetworkUnavailable { protocol })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        net::{SocketAddr, TcpListener},
        sync::{Arc, Mutex},
        thread,
    };

    #[test]
    fn live_tester_authenticates_against_local_protocol_fixtures_without_exposing_secret() {
        let imap = FakeImapServer::start(false);
        let smtp = FakeSmtpServer::start(false, "PLAIN LOGIN");
        let mut tester = LiveGenericAccountConnectionTester {
            timeout: Duration::from_secs(3),
        };

        tester
            .test_generic_account(&config_for(&imap, &smtp, "secret value"))
            .expect("local protocol auth succeeds");

        assert!(imap
            .commands()
            .iter()
            .any(|line| line.starts_with("A002 LOGIN")));
        assert!(smtp
            .commands()
            .iter()
            .any(|line| line.starts_with("AUTH PLAIN ")));
        imap.join();
        smtp.join();
    }

    #[test]
    fn live_tester_rejects_imap_and_smtp_auth_without_echoing_secret() {
        let imap = FakeImapServer::start(true);
        let mut tester = LiveGenericAccountConnectionTester {
            timeout: Duration::from_secs(3),
        };
        let imap_error = tester
            .test_generic_account(&config_for_addrs(
                imap.addr(),
                SocketAddr::from(([127, 0, 0, 1], 9)),
                "secret value",
            ))
            .expect_err("imap auth fails");
        assert_eq!(
            imap_error,
            AccountConnectionTestError::AuthenticationRejected { protocol: "IMAP" }
        );
        assert!(!imap_error.to_string().contains("secret value"));
        imap.join();

        let imap = FakeImapServer::start(false);
        let smtp = FakeSmtpServer::start(true, "LOGIN");
        let smtp_error = tester
            .test_generic_account(&config_for(&imap, &smtp, "another secret"))
            .expect_err("smtp auth fails");
        assert_eq!(
            smtp_error,
            AccountConnectionTestError::AuthenticationRejected { protocol: "SMTP" }
        );
        assert!(!smtp_error.to_string().contains("another secret"));
        imap.join();
        smtp.join();
    }

    #[test]
    fn transport_security_rejects_plaintext_schema() {
        assert_eq!(
            MailTransportSecurity::from_schema("none"),
            Err(AccountConnectionTestError::PlaintextDenied)
        );
        assert_eq!(
            MailTransportSecurity::from_schema("tls").unwrap(),
            MailTransportSecurity::Tls
        );
        assert_eq!(
            MailTransportSecurity::from_schema("starttls").unwrap(),
            MailTransportSecurity::StartTls
        );
    }

    fn config_for(
        imap: &FakeImapServer,
        smtp: &FakeSmtpServer,
        secret: &str,
    ) -> GenericAccountConnectionConfig {
        config_for_addrs(imap.addr(), smtp.addr(), secret)
    }

    fn config_for_addrs(
        imap_addr: SocketAddr,
        smtp_addr: SocketAddr,
        secret: &str,
    ) -> GenericAccountConnectionConfig {
        GenericAccountConnectionConfig {
            username: "user@example.com".into(),
            secret: SecretString::new(secret),
            imap_host: "127.0.0.1".into(),
            imap_port: imap_addr.port(),
            imap_security: MailTransportSecurity::PlaintextForTests,
            smtp_host: "127.0.0.1".into(),
            smtp_port: smtp_addr.port(),
            smtp_security: MailTransportSecurity::PlaintextForTests,
        }
    }

    struct FakeImapServer {
        addr: SocketAddr,
        commands: Arc<Mutex<Vec<String>>>,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl FakeImapServer {
        fn start(reject_auth: bool) -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind imap");
            let addr = listener.local_addr().expect("imap addr");
            let commands = Arc::new(Mutex::new(Vec::new()));
            let thread_commands = Arc::clone(&commands);
            let handle = thread::spawn(move || {
                let (mut stream, _) = listener.accept().expect("accept imap");
                stream
                    .write_all(b"* OK fake IMAP ready\r\n")
                    .expect("write greeting");
                let mut reader = BufReader::new(stream.try_clone().expect("clone imap"));
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).expect("read imap") == 0 {
                        break;
                    }
                    let line = line.trim_end_matches(['\r', '\n']).to_string();
                    thread_commands.lock().expect("commands").push(line.clone());
                    if line.starts_with("A002 LOGIN") {
                        if reject_auth {
                            stream
                                .write_all(b"A002 NO authentication failed\r\n")
                                .expect("write imap no");
                            break;
                        }
                        stream
                            .write_all(b"A002 OK LOGIN completed\r\n")
                            .expect("write imap ok");
                    } else if line.starts_with("A003 LOGOUT") {
                        stream
                            .write_all(b"* BYE logging out\r\nA003 OK LOGOUT completed\r\n")
                            .expect("write logout");
                        break;
                    } else {
                        stream
                            .write_all(b"A000 BAD unsupported\r\n")
                            .expect("write bad");
                    }
                }
            });

            Self {
                addr,
                commands,
                handle: Some(handle),
            }
        }

        fn addr(&self) -> SocketAddr {
            self.addr
        }

        fn commands(&self) -> Vec<String> {
            self.commands.lock().expect("commands").clone()
        }

        fn join(mut self) {
            self.handle.take().expect("imap handle").join().unwrap();
        }
    }

    struct FakeSmtpServer {
        addr: SocketAddr,
        commands: Arc<Mutex<Vec<String>>>,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl FakeSmtpServer {
        fn start(reject_auth: bool, mechanisms: &'static str) -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind smtp");
            let addr = listener.local_addr().expect("smtp addr");
            let commands = Arc::new(Mutex::new(Vec::new()));
            let thread_commands = Arc::clone(&commands);
            let handle = thread::spawn(move || {
                let (mut stream, _) = listener.accept().expect("accept smtp");
                stream
                    .write_all(b"220 fake SMTP ready\r\n")
                    .expect("write greeting");
                let mut reader = BufReader::new(stream.try_clone().expect("clone smtp"));
                let mut login_step = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).expect("read smtp") == 0 {
                        break;
                    }
                    let line = line.trim_end_matches(['\r', '\n']).to_string();
                    thread_commands.lock().expect("commands").push(line.clone());
                    if line.starts_with("EHLO ") {
                        let response =
                            format!("250-fake.smtp\r\n250-AUTH {mechanisms}\r\n250 OK\r\n");
                        stream.write_all(response.as_bytes()).expect("write ehlo");
                    } else if line.starts_with("AUTH PLAIN ") {
                        if reject_auth {
                            stream
                                .write_all(b"535 5.7.8 Authentication credentials invalid\r\n")
                                .expect("write auth no");
                            break;
                        }
                        stream
                            .write_all(b"235 2.7.0 Authentication successful\r\n")
                            .expect("write auth ok");
                    } else if line == "AUTH LOGIN" {
                        login_step = 1;
                        stream
                            .write_all(b"334 VXNlcm5hbWU6\r\n")
                            .expect("login user");
                    } else if login_step == 1 {
                        login_step = 2;
                        stream
                            .write_all(b"334 UGFzc3dvcmQ6\r\n")
                            .expect("login pass");
                    } else if login_step == 2 {
                        if reject_auth {
                            stream
                                .write_all(b"535 5.7.8 Authentication credentials invalid\r\n")
                                .expect("login no");
                            break;
                        }
                        stream
                            .write_all(b"235 2.7.0 Authentication successful\r\n")
                            .expect("login ok");
                    } else if line == "QUIT" {
                        stream.write_all(b"221 2.0.0 Bye\r\n").expect("quit");
                        break;
                    } else {
                        stream
                            .write_all(b"500 unsupported\r\n")
                            .expect("unsupported");
                    }
                }
            });

            Self {
                addr,
                commands,
                handle: Some(handle),
            }
        }

        fn addr(&self) -> SocketAddr {
            self.addr
        }

        fn commands(&self) -> Vec<String> {
            self.commands.lock().expect("commands").clone()
        }

        fn join(mut self) {
            self.handle.take().expect("smtp handle").join().unwrap();
        }
    }
}
