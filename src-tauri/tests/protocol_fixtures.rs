use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};

const READ_TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn fake_imap_protocol_server_supports_reconnect_idle_expunges_and_large_mailboxes() {
    let server = FakeImapProtocolServer::start(FakeImapProtocolConfig {
        mailbox_size: 10_000,
        drop_first_connection: true,
        send_idle_updates: true,
        expunge_on_select: true,
        reject_auth: false,
    });

    let (mut dropped_reader, _) = connect_client(server.addr());
    assert!(read_protocol_line(&mut dropped_reader)
        .expect("read dropped IMAP greeting")
        .contains("Komorebi fake IMAP"));
    assert_eq!(
        read_optional_protocol_line(&mut dropped_reader)
            .expect("read forced disconnect after greeting"),
        None
    );

    let (mut reader, mut writer) = connect_client(server.addr());
    assert!(read_protocol_line(&mut reader)
        .expect("read IMAP greeting")
        .contains("Komorebi fake IMAP"));

    let login = imap_command(&mut reader, &mut writer, "A001", "LOGIN fixture secret")
        .expect("login to fake IMAP");
    assert_eq!(
        login.last().expect("tagged login response"),
        "A001 OK LOGIN completed"
    );

    let select =
        imap_command(&mut reader, &mut writer, "A002", "SELECT INBOX").expect("select inbox");
    assert!(select.iter().any(|line| line == "* 10000 EXISTS"));
    assert!(select
        .iter()
        .any(|line| line == "* OK [UIDVALIDITY 777] valid UIDs"));
    assert!(select
        .iter()
        .any(|line| line == "* OK [UIDNEXT 10001] predicted next UID"));
    assert!(select.iter().any(|line| line == "* 2 EXPUNGE"));

    let search = imap_command(&mut reader, &mut writer, "A003", "UID SEARCH ALL")
        .expect("search large mailbox");
    assert!(search
        .iter()
        .any(|line| line.starts_with("* SEARCH 1 2 3") && line.ends_with(" 10000")));

    let fetch = imap_command(
        &mut reader,
        &mut writer,
        "A004",
        "UID FETCH 10000 (FLAGS RFC822.SIZE BODY.PEEK[HEADER])",
    )
    .expect("fetch high UID from large mailbox");
    assert!(fetch
        .iter()
        .any(|line| line.contains("FETCH (UID 10000 FLAGS (\\Seen)")));

    write_protocol_line(&mut writer, "A005 IDLE").expect("enter IMAP IDLE");
    assert_eq!(
        read_protocol_line(&mut reader).expect("read IDLE continuation"),
        "+ idling"
    );
    assert_eq!(
        read_protocol_line(&mut reader).expect("read IDLE exists update"),
        "* 10001 EXISTS"
    );
    assert_eq!(
        read_protocol_line(&mut reader).expect("read IDLE expunge update"),
        "* 2 EXPUNGE"
    );
    write_protocol_line(&mut writer, "DONE").expect("leave IMAP IDLE");
    assert_eq!(
        read_protocol_line(&mut reader).expect("read IDLE completion"),
        "A005 OK IDLE completed"
    );

    let logout =
        imap_command(&mut reader, &mut writer, "A006", "LOGOUT").expect("logout from fake IMAP");
    assert!(logout.iter().any(|line| line == "* BYE logging out"));
    assert_eq!(
        logout.last().expect("tagged logout response"),
        "A006 OK LOGOUT completed"
    );

    server.join();
}

#[test]
fn fake_imap_protocol_server_can_reject_authentication() {
    let server = FakeImapProtocolServer::start(FakeImapProtocolConfig {
        reject_auth: true,
        ..FakeImapProtocolConfig::default()
    });

    let (mut reader, mut writer) = connect_client(server.addr());
    assert!(read_protocol_line(&mut reader)
        .expect("read IMAP greeting")
        .contains("Komorebi fake IMAP"));

    let login = imap_command(&mut reader, &mut writer, "A001", "LOGIN fixture bad-secret")
        .expect("read failed login response");
    assert_eq!(
        login.last().expect("tagged login response"),
        "A001 NO authentication failed"
    );
    assert_eq!(
        read_optional_protocol_line(&mut reader).expect("auth failure closes connection"),
        None
    );

    server.join();
}

#[test]
fn fake_imap_protocol_server_accepts_sent_append_literal() {
    let server = FakeImapProtocolServer::start(FakeImapProtocolConfig::default());
    let (mut reader, mut writer) = connect_client(server.addr());
    assert!(read_protocol_line(&mut reader)
        .expect("read IMAP greeting")
        .contains("Komorebi fake IMAP"));

    let login = imap_command(&mut reader, &mut writer, "A001", "LOGIN fixture secret")
        .expect("login to fake IMAP");
    assert_eq!(
        login.last().expect("tagged login response"),
        "A001 OK LOGIN completed"
    );
    let message = b"Message-ID: <append@example.com>\r\nSubject: Append\r\n\r\nBody";
    let append = imap_append_command(
        &mut reader,
        &mut writer,
        "A002",
        "APPEND \"Sent Items\" (\\Seen)",
        message,
    )
    .expect("append sent message");
    assert_eq!(
        append.last().expect("tagged append response"),
        "A002 OK [APPENDUID 777 1001] APPEND completed"
    );

    let appended = server.appended_messages();
    assert_eq!(appended.len(), 1);
    assert!(appended[0].contains("Message-ID: <append@example.com>"));
    assert!(appended[0].contains("Subject: Append"));

    let logout =
        imap_command(&mut reader, &mut writer, "A003", "LOGOUT").expect("logout from fake IMAP");
    assert_eq!(
        logout.last().expect("tagged logout response"),
        "A003 OK LOGOUT completed"
    );
    server.join();
}

#[test]
fn fake_smtp_protocol_server_accepts_message_and_records_data() {
    let server = FakeSmtpProtocolServer::start(FakeSmtpProtocolConfig::default());
    let (mut reader, mut writer) = connect_client(server.addr());

    assert_eq!(
        smtp_response(&mut reader).expect("read SMTP greeting"),
        vec!["220 komorebi.test fake SMTP ready"]
    );
    write_protocol_line(&mut writer, "EHLO komorebi.local").expect("send EHLO");
    let ehlo = smtp_response(&mut reader).expect("read EHLO response");
    assert_eq!(ehlo[0], "250-komorebi.test");
    assert!(ehlo.iter().any(|line| line == "250-AUTH PLAIN LOGIN"));

    write_protocol_line(&mut writer, "AUTH PLAIN AHVzZXIAcGFzcw==").expect("send AUTH");
    assert_eq!(
        smtp_response(&mut reader).expect("read AUTH success"),
        vec!["235 2.7.0 Authentication successful"]
    );
    write_protocol_line(&mut writer, "MAIL FROM:<sender@example.com>").expect("send MAIL FROM");
    assert_eq!(
        smtp_response(&mut reader).expect("read MAIL FROM response"),
        vec!["250 2.1.0 OK"]
    );
    write_protocol_line(&mut writer, "RCPT TO:<recipient@example.com>").expect("send RCPT TO");
    assert_eq!(
        smtp_response(&mut reader).expect("read RCPT TO response"),
        vec!["250 2.1.5 OK"]
    );
    write_protocol_line(&mut writer, "DATA").expect("send DATA");
    assert_eq!(
        smtp_response(&mut reader).expect("read DATA response"),
        vec!["354 End data with <CR><LF>.<CR><LF>"]
    );
    writer
        .write_all(b"Subject: Protocol fixture\r\n\r\nThis message stayed on localhost.\r\n.\r\n")
        .expect("write SMTP message data");
    writer.flush().expect("flush SMTP data");
    assert_eq!(
        smtp_response(&mut reader).expect("read queued response"),
        vec!["250 2.0.0 queued"]
    );
    write_protocol_line(&mut writer, "QUIT").expect("send QUIT");
    assert_eq!(
        smtp_response(&mut reader).expect("read QUIT response"),
        vec!["221 2.0.0 Bye"]
    );

    let messages = server.recorded_messages();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("Subject: Protocol fixture"));
    assert!(messages[0].contains("This message stayed on localhost."));

    server.join();
}

#[test]
fn fake_smtp_protocol_server_can_reject_authentication() {
    let server = FakeSmtpProtocolServer::start(FakeSmtpProtocolConfig { reject_auth: true });
    let (mut reader, mut writer) = connect_client(server.addr());

    assert_eq!(
        smtp_response(&mut reader).expect("read SMTP greeting"),
        vec!["220 komorebi.test fake SMTP ready"]
    );
    write_protocol_line(&mut writer, "EHLO komorebi.local").expect("send EHLO");
    assert!(smtp_response(&mut reader)
        .expect("read EHLO response")
        .iter()
        .any(|line| line == "250-AUTH PLAIN LOGIN"));

    write_protocol_line(&mut writer, "AUTH PLAIN AHVzZXIAcGFzcw==").expect("send AUTH");
    assert_eq!(
        smtp_response(&mut reader).expect("read AUTH failure"),
        vec!["535 5.7.8 Authentication credentials invalid"]
    );
    assert_eq!(
        read_optional_protocol_line(&mut reader).expect("auth failure closes SMTP connection"),
        None
    );
    assert!(server.recorded_messages().is_empty());

    server.join();
}

#[derive(Clone)]
struct FakeImapProtocolConfig {
    mailbox_size: usize,
    drop_first_connection: bool,
    send_idle_updates: bool,
    expunge_on_select: bool,
    reject_auth: bool,
}

impl Default for FakeImapProtocolConfig {
    fn default() -> Self {
        Self {
            mailbox_size: 3,
            drop_first_connection: false,
            send_idle_updates: false,
            expunge_on_select: false,
            reject_auth: false,
        }
    }
}

struct FakeImapProtocolServer {
    addr: SocketAddr,
    handle: Option<JoinHandle<()>>,
    appended_messages: Arc<Mutex<Vec<String>>>,
}

impl FakeImapProtocolServer {
    fn start(config: FakeImapProtocolConfig) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind fake IMAP listener");
        let addr = listener
            .local_addr()
            .expect("read fake IMAP listener address");
        let connection_count = if config.drop_first_connection { 2 } else { 1 };
        let appended_messages = Arc::new(Mutex::new(Vec::new()));
        let server_appended_messages = Arc::clone(&appended_messages);

        let handle = thread::spawn(move || {
            for connection_index in 0..connection_count {
                let (stream, _) = listener.accept().expect("accept fake IMAP connection");
                handle_fake_imap_connection(
                    stream,
                    &config,
                    connection_index,
                    &server_appended_messages,
                )
                .expect("serve fake IMAP connection");
            }
        });

        Self {
            addr,
            handle: Some(handle),
            appended_messages,
        }
    }

    fn addr(&self) -> SocketAddr {
        self.addr
    }

    fn appended_messages(&self) -> Vec<String> {
        self.appended_messages
            .lock()
            .expect("fake IMAP appended messages lock")
            .clone()
    }

    fn join(mut self) {
        self.handle
            .take()
            .expect("fake IMAP join handle")
            .join()
            .expect("fake IMAP thread exits cleanly");
    }
}

fn handle_fake_imap_connection(
    stream: TcpStream,
    config: &FakeImapProtocolConfig,
    connection_index: usize,
    appended_messages: &Arc<Mutex<Vec<String>>>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_write_timeout(Some(READ_TIMEOUT))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);

    write_protocol_line(&mut writer, "* OK Komorebi fake IMAP ready")?;
    if config.drop_first_connection && connection_index == 0 {
        return Ok(());
    }

    loop {
        let Some(line) = read_optional_protocol_line(&mut reader)? else {
            return Ok(());
        };
        let Some((tag, command)) = line.split_once(' ') else {
            write_protocol_line(&mut writer, "* BAD malformed command")?;
            continue;
        };
        let command_upper = command.to_ascii_uppercase();

        if command_upper.starts_with("LOGIN") {
            if config.reject_auth {
                write_protocol_line(&mut writer, &format!("{tag} NO authentication failed"))?;
                return Ok(());
            }
            write_protocol_line(&mut writer, &format!("{tag} OK LOGIN completed"))?;
        } else if command_upper == "SELECT INBOX" || command_upper == "SELECT \"SENT ITEMS\"" {
            write_protocol_line(
                &mut writer,
                "* FLAGS (\\Seen \\Flagged \\Answered \\Deleted)",
            )?;
            write_protocol_line(&mut writer, &format!("* {} EXISTS", config.mailbox_size))?;
            if config.expunge_on_select {
                write_protocol_line(&mut writer, "* 2 EXPUNGE")?;
            }
            write_protocol_line(&mut writer, "* OK [UIDVALIDITY 777] valid UIDs")?;
            write_protocol_line(
                &mut writer,
                &format!(
                    "* OK [UIDNEXT {}] predicted next UID",
                    config.mailbox_size + 1
                ),
            )?;
            write_protocol_line(
                &mut writer,
                &format!("{tag} OK [READ-WRITE] SELECT completed"),
            )?;
        } else if command_upper == "UID SEARCH ALL" {
            let mut search_response = String::from("* SEARCH");
            for uid in 1..=config.mailbox_size {
                search_response.push(' ');
                search_response.push_str(&uid.to_string());
            }
            write_protocol_line(&mut writer, &search_response)?;
            write_protocol_line(&mut writer, &format!("{tag} OK SEARCH completed"))?;
        } else if command_upper.starts_with("UID FETCH ") {
            for uid in requested_fetch_uids(command, config.mailbox_size) {
                write_protocol_line(
                    &mut writer,
                    &format!(
                        "* {uid} FETCH (UID {uid} FLAGS (\\Seen) RFC822.SIZE 42 BODY[HEADER] \"Subject: Large mailbox {uid}\")"
                    ),
                )?;
            }
            write_protocol_line(&mut writer, &format!("{tag} OK FETCH completed"))?;
        } else if command_upper.starts_with("APPEND ") {
            let literal_len = parse_literal_len(command).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "APPEND omitted literal length")
            })?;
            write_protocol_line(&mut writer, "+ ready for literal")?;
            let mut bytes = vec![0_u8; literal_len];
            reader.read_exact(&mut bytes)?;
            let mut crlf = [0_u8; 2];
            reader.read_exact(&mut crlf)?;
            if &crlf != b"\r\n" {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "APPEND literal missing CRLF",
                ));
            }
            appended_messages
                .lock()
                .expect("fake IMAP appended messages lock")
                .push(String::from_utf8_lossy(&bytes).into_owned());
            write_protocol_line(
                &mut writer,
                &format!("{tag} OK [APPENDUID 777 1001] APPEND completed"),
            )?;
        } else if command_upper == "IDLE" {
            write_protocol_line(&mut writer, "+ idling")?;
            if config.send_idle_updates {
                write_protocol_line(
                    &mut writer,
                    &format!("* {} EXISTS", config.mailbox_size + 1),
                )?;
                write_protocol_line(&mut writer, "* 2 EXPUNGE")?;
            }
            let done = read_protocol_line(&mut reader)?;
            if done.eq_ignore_ascii_case("DONE") {
                write_protocol_line(&mut writer, &format!("{tag} OK IDLE completed"))?;
            } else {
                write_protocol_line(&mut writer, &format!("{tag} BAD IDLE expected DONE"))?;
            }
        } else if command_upper == "LOGOUT" {
            write_protocol_line(&mut writer, "* BYE logging out")?;
            write_protocol_line(&mut writer, &format!("{tag} OK LOGOUT completed"))?;
            return Ok(());
        } else {
            write_protocol_line(&mut writer, &format!("{tag} NO unsupported command"))?;
        }
    }
}

fn parse_literal_len(command: &str) -> Option<usize> {
    let trimmed = command.trim_end();
    let marker_start = trimmed.rfind('{')?;
    let marker_end = trimmed.rfind('}')?;
    if marker_end <= marker_start {
        return None;
    }
    trimmed[marker_start + 1..marker_end].parse().ok()
}

fn requested_fetch_uids(command: &str, mailbox_size: usize) -> Vec<usize> {
    let Some(rest) = command.get("UID FETCH ".len()..) else {
        return Vec::new();
    };
    let sequence = rest.split_whitespace().next().unwrap_or_default();
    if sequence.contains(':') || sequence == "*" {
        return (1..=mailbox_size).collect();
    }

    sequence
        .parse::<usize>()
        .ok()
        .filter(|uid| (1..=mailbox_size).contains(uid))
        .into_iter()
        .collect()
}

#[derive(Clone, Default)]
struct FakeSmtpProtocolConfig {
    reject_auth: bool,
}

struct FakeSmtpProtocolServer {
    addr: SocketAddr,
    handle: Option<JoinHandle<()>>,
    recorded_messages: Arc<Mutex<Vec<String>>>,
}

impl FakeSmtpProtocolServer {
    fn start(config: FakeSmtpProtocolConfig) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind fake SMTP listener");
        let addr = listener
            .local_addr()
            .expect("read fake SMTP listener address");
        let recorded_messages = Arc::new(Mutex::new(Vec::new()));
        let server_messages = Arc::clone(&recorded_messages);

        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept fake SMTP connection");
            handle_fake_smtp_connection(stream, &config, &server_messages)
                .expect("serve fake SMTP connection");
        });

        Self {
            addr,
            handle: Some(handle),
            recorded_messages,
        }
    }

    fn addr(&self) -> SocketAddr {
        self.addr
    }

    fn recorded_messages(&self) -> Vec<String> {
        self.recorded_messages
            .lock()
            .expect("fake SMTP recorded messages lock")
            .clone()
    }

    fn join(mut self) {
        self.handle
            .take()
            .expect("fake SMTP join handle")
            .join()
            .expect("fake SMTP thread exits cleanly");
    }
}

fn handle_fake_smtp_connection(
    stream: TcpStream,
    config: &FakeSmtpProtocolConfig,
    recorded_messages: &Arc<Mutex<Vec<String>>>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_write_timeout(Some(READ_TIMEOUT))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);

    write_protocol_line(&mut writer, "220 komorebi.test fake SMTP ready")?;
    loop {
        let Some(line) = read_optional_protocol_line(&mut reader)? else {
            return Ok(());
        };
        let command_upper = line.to_ascii_uppercase();
        if command_upper.starts_with("EHLO ") || command_upper.starts_with("HELO ") {
            write_protocol_line(&mut writer, "250-komorebi.test")?;
            write_protocol_line(&mut writer, "250-AUTH PLAIN LOGIN")?;
            write_protocol_line(&mut writer, "250 SIZE 10485760")?;
        } else if command_upper.starts_with("AUTH ") {
            if config.reject_auth {
                write_protocol_line(&mut writer, "535 5.7.8 Authentication credentials invalid")?;
                return Ok(());
            }
            write_protocol_line(&mut writer, "235 2.7.0 Authentication successful")?;
        } else if command_upper.starts_with("MAIL FROM:") {
            write_protocol_line(&mut writer, "250 2.1.0 OK")?;
        } else if command_upper.starts_with("RCPT TO:") {
            write_protocol_line(&mut writer, "250 2.1.5 OK")?;
        } else if command_upper == "DATA" {
            write_protocol_line(&mut writer, "354 End data with <CR><LF>.<CR><LF>")?;
            let mut message = String::new();
            loop {
                let data_line = read_protocol_line(&mut reader)?;
                if data_line == "." {
                    break;
                }
                message.push_str(&data_line);
                message.push_str("\r\n");
            }
            recorded_messages
                .lock()
                .expect("fake SMTP recorded messages lock")
                .push(message);
            write_protocol_line(&mut writer, "250 2.0.0 queued")?;
        } else if command_upper == "QUIT" {
            write_protocol_line(&mut writer, "221 2.0.0 Bye")?;
            return Ok(());
        } else {
            write_protocol_line(&mut writer, "502 5.5.2 Command not recognized")?;
        }
    }
}

fn connect_client(addr: SocketAddr) -> (BufReader<TcpStream>, TcpStream) {
    let stream = TcpStream::connect(addr).expect("connect to fake protocol server");
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .expect("set client read timeout");
    stream
        .set_write_timeout(Some(READ_TIMEOUT))
        .expect("set client write timeout");
    let writer = stream.try_clone().expect("clone fake protocol stream");
    (BufReader::new(stream), writer)
}

fn imap_command(
    reader: &mut BufReader<TcpStream>,
    writer: &mut TcpStream,
    tag: &str,
    command: &str,
) -> io::Result<Vec<String>> {
    write_protocol_line(writer, &format!("{tag} {command}"))?;
    let mut response = Vec::new();
    loop {
        let line = read_protocol_line(reader)?;
        let tagged = line.starts_with(tag);
        response.push(line);
        if tagged {
            return Ok(response);
        }
    }
}

fn imap_append_command(
    reader: &mut BufReader<TcpStream>,
    writer: &mut TcpStream,
    tag: &str,
    command_prefix: &str,
    literal: &[u8],
) -> io::Result<Vec<String>> {
    write_protocol_line(
        writer,
        &format!("{tag} {command_prefix} {{{}}}", literal.len()),
    )?;
    let continuation = read_protocol_line(reader)?;
    if !continuation.starts_with('+') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "APPEND continuation was not returned",
        ));
    }
    writer.write_all(literal)?;
    writer.write_all(b"\r\n")?;
    writer.flush()?;

    let mut response = Vec::new();
    loop {
        let line = read_protocol_line(reader)?;
        let tagged = line.starts_with(tag);
        response.push(line);
        if tagged {
            return Ok(response);
        }
    }
}

fn smtp_response(reader: &mut BufReader<TcpStream>) -> io::Result<Vec<String>> {
    let first = read_protocol_line(reader)?;
    let code = first
        .get(..3)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "short SMTP response"))?
        .to_string();
    let is_multiline = first.as_bytes().get(3) == Some(&b'-');
    let mut lines = vec![first];

    if is_multiline {
        loop {
            let line = read_protocol_line(reader)?;
            let is_final = line.starts_with(&code) && line.as_bytes().get(3) == Some(&b' ');
            lines.push(line);
            if is_final {
                break;
            }
        }
    }

    Ok(lines)
}

fn read_protocol_line(reader: &mut BufReader<TcpStream>) -> io::Result<String> {
    read_optional_protocol_line(reader)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "connection closed"))
}

fn read_optional_protocol_line(reader: &mut BufReader<TcpStream>) -> io::Result<Option<String>> {
    let mut line = String::new();
    let bytes_read = reader.read_line(&mut line)?;
    if bytes_read == 0 {
        return Ok(None);
    }

    Ok(Some(trim_protocol_line(&line).to_string()))
}

fn write_protocol_line(writer: &mut TcpStream, line: &str) -> io::Result<()> {
    writer.write_all(line.as_bytes())?;
    writer.write_all(b"\r\n")?;
    writer.flush()
}

fn trim_protocol_line(line: &str) -> &str {
    line.trim_end_matches(&['\r', '\n'][..])
}
