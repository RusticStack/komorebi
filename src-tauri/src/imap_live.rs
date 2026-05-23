use std::{
    collections::BTreeSet,
    fmt,
    io::{self, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};

use native_tls::{TlsConnector, TlsStream};

use crate::{
    account_connection::MailTransportSecurity,
    imap_sync::{
        IdleCapability, IdleWaitOutcome, ImapIdleServer, ImapSyncServer, RemoteFlag,
        RemoteFlagUpdate, RemoteFolder, RemoteFolderStatus, RemoteMessage, SyncCancellationToken,
        SyncError,
    },
    mime_normalization::{parse_inbound_mime, ParseInboundMimeRequest},
    oauth::build_xoauth2_initial_response,
    token_lifecycle::SecretString,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_PROTOCOL_LINE_BYTES: usize = 1024 * 1024;
const DEFAULT_MAX_MESSAGE_BYTES: usize = 25 * 1024 * 1024;
const FETCH_MESSAGE_CHUNK_SIZE: usize = 50;
const FETCH_FLAG_CHUNK_SIZE: usize = 500;
const IDLE_READ_SLICE: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveImapAuth {
    Password {
        username: String,
        secret: SecretString,
    },
    XOAuth2 {
        username: String,
        access_token: SecretString,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveImapConfig {
    pub account_id: String,
    pub host: String,
    pub port: u16,
    pub security: MailTransportSecurity,
    pub auth: LiveImapAuth,
    pub timeout: Duration,
    pub max_message_bytes: usize,
}

impl LiveImapConfig {
    pub fn new(
        account_id: impl Into<String>,
        host: impl Into<String>,
        port: u16,
        security: MailTransportSecurity,
        auth: LiveImapAuth,
    ) -> Self {
        Self {
            account_id: account_id.into(),
            host: host.into(),
            port,
            security,
            auth,
            timeout: DEFAULT_TIMEOUT,
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_max_message_bytes(mut self, max_message_bytes: usize) -> Self {
        self.max_message_bytes = max_message_bytes;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveImapStoreMode {
    Add,
    Remove,
    Replace,
}

impl LiveImapStoreMode {
    fn command(self) -> &'static str {
        match self {
            Self::Add => "+FLAGS",
            Self::Remove => "-FLAGS",
            Self::Replace => "FLAGS",
        }
    }
}

#[derive(Debug)]
pub struct LiveImapAdapter {
    config: LiveImapConfig,
    connection: Option<ImapConnection>,
}

impl LiveImapAdapter {
    pub fn connect(config: LiveImapConfig) -> Result<Self, SyncError> {
        let mut adapter = Self {
            config,
            connection: None,
        };
        adapter.open_authenticated_connection()?;
        Ok(adapter)
    }

    pub fn config(&self) -> &LiveImapConfig {
        &self.config
    }

    pub fn logout(&mut self) -> Result<(), SyncError> {
        if let Some(connection) = self.connection.as_mut() {
            let _ = connection.run_command(
                "LOGOUT",
                self.config.timeout,
                self.config.max_message_bytes,
            );
        }
        self.connection = None;
        Ok(())
    }

    pub fn store_flags(
        &mut self,
        account_id: &str,
        remote_id: &str,
        uids: &[u64],
        mode: LiveImapStoreMode,
        flags: &BTreeSet<RemoteFlag>,
    ) -> Result<Vec<RemoteFlagUpdate>, SyncError> {
        self.ensure_account(account_id)?;
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        if flags.is_empty() {
            return Err(SyncError::InvalidRemoteState {
                reason: "IMAP STORE requires at least one flag",
            });
        }

        self.ensure_selected(remote_id)?;
        let uid_set = uid_sequence_set(uids)?;
        let flag_list = imap_flag_list(flags);
        let command = format!("UID STORE {uid_set} {} {flag_list}", mode.command());
        let timeout = self.config.timeout;
        let max_message_bytes = self.config.max_message_bytes;
        let responses = self
            .connection_mut()?
            .run_command(&command, timeout, max_message_bytes)?;
        let uid_validity = self.selected_uid_validity(remote_id)?;
        let mut updates = parse_flag_updates(&responses, uid_validity)?;
        if updates.is_empty() {
            updates = self.fetch_flags(account_id, remote_id, uid_validity, uids)?;
        }

        Ok(updates)
    }

    pub fn expunge_folder(
        &mut self,
        account_id: &str,
        remote_id: &str,
    ) -> Result<usize, SyncError> {
        self.ensure_account(account_id)?;
        self.ensure_selected(remote_id)?;
        let timeout = self.config.timeout;
        let max_message_bytes = self.config.max_message_bytes;
        let responses =
            self.connection_mut()?
                .run_command("EXPUNGE", timeout, max_message_bytes)?;

        Ok(responses
            .iter()
            .filter(|response| {
                let text = response.text();
                text.starts_with("* ") && text.to_ascii_uppercase().contains(" EXPUNGE")
            })
            .count())
    }

    fn open_authenticated_connection(&mut self) -> Result<(), SyncError> {
        let stream = open_stream(&self.config)?;
        let mut connection = ImapConnection::new(stream);

        match &self.config.auth {
            LiveImapAuth::Password { username, secret } => {
                let username = quote_imap_string(username)?;
                let password = quote_imap_string(secret.expose_secret())?;
                let command = format!("LOGIN {username} {password}");
                connection.run_auth_command(
                    &command,
                    self.config.timeout,
                    self.config.max_message_bytes,
                )?;
            }
            LiveImapAuth::XOAuth2 {
                username,
                access_token,
            } => {
                let response = build_xoauth2_initial_response(username, access_token)
                    .map_err(|_| SyncError::permanent("IMAP OAuth response could not be built"))?;
                connection.authenticate_xoauth2(
                    &response,
                    self.config.timeout,
                    self.config.max_message_bytes,
                )?;
            }
        }

        self.connection = Some(connection);
        Ok(())
    }

    fn ensure_account(&self, account_id: &str) -> Result<(), SyncError> {
        if self.config.account_id == account_id {
            Ok(())
        } else {
            Err(SyncError::UnknownAccount)
        }
    }

    fn connection_mut(&mut self) -> Result<&mut ImapConnection, SyncError> {
        if self.connection.is_none() {
            self.open_authenticated_connection()?;
        }

        self.connection
            .as_mut()
            .ok_or_else(|| SyncError::transient("IMAP connection is unavailable"))
    }

    fn ensure_selected(&mut self, remote_id: &str) -> Result<(), SyncError> {
        let selected = self
            .connection
            .as_ref()
            .and_then(|connection| connection.selected_remote_id.as_deref());
        if selected == Some(remote_id) {
            return Ok(());
        }

        self.select_folder_inner(remote_id).map(|_| ())
    }

    fn selected_uid_validity(&mut self, remote_id: &str) -> Result<u64, SyncError> {
        self.ensure_selected(remote_id)?;
        self.connection
            .as_ref()
            .and_then(|connection| connection.selected_uid_validity)
            .ok_or(SyncError::InvalidRemoteState {
                reason: "selected IMAP folder did not report UIDVALIDITY",
            })
    }

    fn select_folder_inner(&mut self, remote_id: &str) -> Result<RemoteFolderStatus, SyncError> {
        let quoted = quote_imap_string(remote_id)?;
        let command = format!("SELECT {quoted}");
        let timeout = self.config.timeout;
        let max_message_bytes = self.config.max_message_bytes;
        let responses = self
            .connection_mut()?
            .run_command(&command, timeout, max_message_bytes)?;
        let uid_validity = parse_status_number(&responses, "UIDVALIDITY").ok_or(
            SyncError::InvalidRemoteState {
                reason: "selected IMAP folder omitted UIDVALIDITY",
            },
        )?;
        let uid_next =
            parse_status_number(&responses, "UIDNEXT").ok_or(SyncError::InvalidRemoteState {
                reason: "selected IMAP folder omitted UIDNEXT",
            })?;
        let total_count = parse_exists_count(&responses)?;

        {
            let connection = self.connection_mut()?;
            connection.selected_remote_id = Some(remote_id.to_string());
            connection.selected_uid_validity = Some(uid_validity);
        }

        let existing_uids = self.uid_search("ALL")?;
        let unread_count = self.uid_search("UNSEEN")?.len().try_into().map_err(|_| {
            SyncError::InvalidRemoteState {
                reason: "IMAP unread count overflowed",
            }
        })?;

        Ok(RemoteFolderStatus {
            uid_validity,
            uid_next,
            existing_uids,
            unread_count,
            total_count,
        })
    }

    fn uid_search(&mut self, query: &str) -> Result<BTreeSet<u64>, SyncError> {
        let command = format!("UID SEARCH {query}");
        let timeout = self.config.timeout;
        let max_message_bytes = self.config.max_message_bytes;
        let responses = self
            .connection_mut()?
            .run_command(&command, timeout, max_message_bytes)?;
        Ok(parse_search_uids(&responses))
    }
}

impl ImapSyncServer for LiveImapAdapter {
    fn list_folders(&mut self, account_id: &str) -> Result<Vec<RemoteFolder>, SyncError> {
        self.ensure_account(account_id)?;
        let timeout = self.config.timeout;
        let max_message_bytes = self.config.max_message_bytes;
        let responses =
            self.connection_mut()?
                .run_command(r#"LIST "" "*""#, timeout, max_message_bytes)?;
        let folders = responses
            .iter()
            .filter_map(|response| parse_list_response(&response.text()).transpose())
            .collect::<Result<Vec<_>, SyncError>>()?;

        if folders.is_empty() {
            return Err(SyncError::InvalidRemoteState {
                reason: "IMAP server returned no selectable folders",
            });
        }

        Ok(folders)
    }

    fn select_folder(
        &mut self,
        account_id: &str,
        remote_id: &str,
    ) -> Result<RemoteFolderStatus, SyncError> {
        self.ensure_account(account_id)?;
        self.select_folder_inner(remote_id)
    }

    fn fetch_messages(
        &mut self,
        account_id: &str,
        remote_id: &str,
        folder_id: &str,
        uid_validity: u64,
        uids: &[u64],
    ) -> Result<Vec<RemoteMessage>, SyncError> {
        self.ensure_account(account_id)?;
        if uids.is_empty() {
            return Ok(Vec::new());
        }

        self.ensure_selected(remote_id)?;
        let mut messages = Vec::new();
        for chunk in uids.chunks(FETCH_MESSAGE_CHUNK_SIZE) {
            let uid_set = uid_sequence_set(chunk)?;
            let command =
                format!("UID FETCH {uid_set} (UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[])");
            let timeout = self.config.timeout;
            let max_message_bytes = self.config.max_message_bytes;
            let responses =
                self.connection_mut()?
                    .run_command(&command, timeout, max_message_bytes)?;
            for response in responses
                .iter()
                .filter(|response| is_untagged_fetch(&response.text()))
            {
                messages.push(parse_message_fetch_response(
                    account_id,
                    folder_id,
                    response,
                    uid_validity,
                )?);
            }
        }

        Ok(messages)
    }

    fn fetch_flags(
        &mut self,
        account_id: &str,
        remote_id: &str,
        uid_validity: u64,
        uids: &[u64],
    ) -> Result<Vec<RemoteFlagUpdate>, SyncError> {
        self.ensure_account(account_id)?;
        if uids.is_empty() {
            return Ok(Vec::new());
        }

        self.ensure_selected(remote_id)?;
        let mut updates = Vec::new();
        for chunk in uids.chunks(FETCH_FLAG_CHUNK_SIZE) {
            let uid_set = uid_sequence_set(chunk)?;
            let command = format!("UID FETCH {uid_set} (UID FLAGS)");
            let timeout = self.config.timeout;
            let max_message_bytes = self.config.max_message_bytes;
            let responses =
                self.connection_mut()?
                    .run_command(&command, timeout, max_message_bytes)?;
            updates.extend(parse_flag_updates(&responses, uid_validity)?);
        }

        Ok(updates)
    }

    fn reconnect(&mut self, account_id: &str) -> Result<(), SyncError> {
        self.ensure_account(account_id)?;
        self.connection = None;
        self.open_authenticated_connection()
    }
}

impl ImapIdleServer for LiveImapAdapter {
    fn idle_capability(
        &mut self,
        account_id: &str,
        _remote_id: &str,
    ) -> Result<IdleCapability, SyncError> {
        self.ensure_account(account_id)?;
        let timeout = self.config.timeout;
        let max_message_bytes = self.config.max_message_bytes;
        let responses =
            self.connection_mut()?
                .run_command("CAPABILITY", timeout, max_message_bytes)?;
        let has_idle = responses.iter().any(|response| {
            response
                .text()
                .split_whitespace()
                .any(|token| token.eq_ignore_ascii_case("IDLE"))
        });

        Ok(if has_idle {
            IdleCapability::Available
        } else {
            IdleCapability::Unavailable
        })
    }

    fn idle_wait(
        &mut self,
        account_id: &str,
        remote_id: &str,
        timeout: Duration,
        cancellation: &SyncCancellationToken,
    ) -> Result<IdleWaitOutcome, SyncError> {
        self.ensure_account(account_id)?;
        if cancellation.is_cancelled() {
            return Err(SyncError::Cancelled);
        }
        self.ensure_selected(remote_id)?;

        let max_message_bytes = self.config.max_message_bytes;
        let command_timeout = self.config.timeout;
        let connection = self.connection_mut()?;
        let tag = connection.send_tagged("IDLE")?;
        connection.set_read_timeout(Some(command_timeout))?;
        let first = connection.read_response(max_message_bytes)?;
        if first.line.starts_with('+') {
            // IDLE accepted.
        } else if first.is_tagged(&tag) {
            return Ok(IdleWaitOutcome::IdleUnavailable);
        } else {
            return Err(SyncError::InvalidRemoteState {
                reason: "IMAP IDLE did not return a continuation",
            });
        }

        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        loop {
            if cancellation.is_cancelled() {
                let _ = connection.finish_idle(&tag, command_timeout, max_message_bytes);
                return Err(SyncError::Cancelled);
            }

            let now = Instant::now();
            if now >= deadline {
                connection.finish_idle(&tag, command_timeout, max_message_bytes)?;
                return Ok(IdleWaitOutcome::Timeout);
            }

            let remaining = deadline.saturating_duration_since(now);
            connection.set_read_timeout(Some(remaining.min(IDLE_READ_SLICE)))?;
            match connection.read_response_result(max_message_bytes) {
                Ok(response) if response.is_mailbox_change() => {
                    connection.finish_idle(&tag, command_timeout, max_message_bytes)?;
                    return Ok(IdleWaitOutcome::MailboxChanged);
                }
                Ok(response) if response.is_tagged(&tag) => {
                    return Ok(IdleWaitOutcome::IdleUnavailable);
                }
                Ok(_) => {}
                Err(ImapReadError::TimedOut) => {}
                Err(error) => return Err(error.into_sync_error()),
            }
        }
    }
}

#[derive(Debug)]
struct ImapConnection {
    stream: Box<dyn ImapProtocolStream>,
    next_tag: u32,
    selected_remote_id: Option<String>,
    selected_uid_validity: Option<u64>,
}

impl ImapConnection {
    fn new(stream: Box<dyn ImapProtocolStream>) -> Self {
        Self {
            stream,
            next_tag: 1,
            selected_remote_id: None,
            selected_uid_validity: None,
        }
    }

    fn run_command(
        &mut self,
        command: &str,
        timeout: Duration,
        max_literal_bytes: usize,
    ) -> Result<Vec<ImapResponse>, SyncError> {
        let tag = self.send_tagged(command)?;
        let responses = self.read_until_tag(&tag, timeout, max_literal_bytes)?;
        require_tagged_ok(
            &responses,
            SyncError::permanent("IMAP command was rejected"),
        )?;
        Ok(responses)
    }

    fn run_auth_command(
        &mut self,
        command: &str,
        timeout: Duration,
        max_literal_bytes: usize,
    ) -> Result<(), SyncError> {
        let tag = self.send_tagged(command)?;
        let responses = self.read_until_tag(&tag, timeout, max_literal_bytes)?;
        require_tagged_ok(
            &responses,
            SyncError::permanent("IMAP authentication was rejected"),
        )
    }

    fn authenticate_xoauth2(
        &mut self,
        initial_response: &str,
        timeout: Duration,
        max_literal_bytes: usize,
    ) -> Result<(), SyncError> {
        let tag = self.send_tagged("AUTHENTICATE XOAUTH2")?;
        self.set_read_timeout(Some(timeout))?;
        let first = self.read_response(max_literal_bytes)?;
        if !first.line.starts_with('+') {
            if first.is_tagged(&tag) {
                require_tagged_ok(
                    &[first],
                    SyncError::permanent("IMAP authentication was rejected"),
                )?;
                return Ok(());
            }
            return Err(SyncError::InvalidRemoteState {
                reason: "IMAP OAuth authentication did not return a continuation",
            });
        }

        self.send_untagged(initial_response)?;
        let responses = self.read_until_tag(&tag, timeout, max_literal_bytes)?;
        require_tagged_ok(
            &responses,
            SyncError::permanent("IMAP authentication was rejected"),
        )
    }

    fn send_tagged(&mut self, command: &str) -> Result<String, SyncError> {
        let tag = format!("A{:04}", self.next_tag);
        self.next_tag = self.next_tag.saturating_add(1);
        write_protocol_line(self.stream.as_mut(), &format!("{tag} {command}"))?;
        Ok(tag)
    }

    fn send_untagged(&mut self, line: &str) -> Result<(), SyncError> {
        write_protocol_line(self.stream.as_mut(), line)
    }

    fn read_until_tag(
        &mut self,
        tag: &str,
        timeout: Duration,
        max_literal_bytes: usize,
    ) -> Result<Vec<ImapResponse>, SyncError> {
        self.set_read_timeout(Some(timeout))?;
        let mut responses = Vec::new();
        loop {
            let response = self.read_response(max_literal_bytes)?;
            let tagged = response.is_tagged(tag);
            responses.push(response);
            if tagged {
                return Ok(responses);
            }
        }
    }

    fn read_response(&mut self, max_literal_bytes: usize) -> Result<ImapResponse, SyncError> {
        self.read_response_result(max_literal_bytes)
            .map_err(ImapReadError::into_sync_error)
    }

    fn read_response_result(
        &mut self,
        max_literal_bytes: usize,
    ) -> Result<ImapResponse, ImapReadError> {
        read_response(self.stream.as_mut(), max_literal_bytes)
    }

    fn finish_idle(
        &mut self,
        tag: &str,
        timeout: Duration,
        max_literal_bytes: usize,
    ) -> Result<(), SyncError> {
        self.send_untagged("DONE")?;
        let responses = self.read_until_tag(tag, timeout, max_literal_bytes)?;
        require_tagged_ok(&responses, SyncError::permanent("IMAP IDLE was rejected"))
    }

    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<(), SyncError> {
        self.stream
            .set_read_timeout(timeout)
            .map_err(|_| SyncError::transient("IMAP network operation failed"))
    }
}

trait ImapProtocolStream: Read + Write + Send + fmt::Debug {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()>;
}

impl ImapProtocolStream for TcpStream {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        TcpStream::set_read_timeout(self, timeout)
    }
}

impl ImapProtocolStream for TlsStream<TcpStream> {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.get_ref().set_read_timeout(timeout)
    }
}

fn open_stream(config: &LiveImapConfig) -> Result<Box<dyn ImapProtocolStream>, SyncError> {
    let connector = TlsConnector::builder()
        .build()
        .map_err(|_| SyncError::transient("IMAP TLS negotiation failed"))?;
    let mut tcp = connect_tcp(&config.host, config.port, config.timeout)?;

    match config.security {
        MailTransportSecurity::Tls => {
            let mut tls = connector
                .connect(&config.host, tcp)
                .map_err(|_| SyncError::transient("IMAP TLS negotiation failed"))?;
            read_greeting(&mut tls, config.max_message_bytes)?;
            Ok(Box::new(tls))
        }
        MailTransportSecurity::StartTls => {
            read_greeting(&mut tcp, config.max_message_bytes)?;
            write_protocol_line(&mut tcp, "A0000 STARTTLS")?;
            let responses =
                read_until_tag_raw(&mut tcp, "A0000", config.timeout, config.max_message_bytes)?;
            require_tagged_ok(
                &responses,
                SyncError::permanent("IMAP STARTTLS was not accepted"),
            )?;
            let tls = connector
                .connect(&config.host, tcp)
                .map_err(|_| SyncError::transient("IMAP TLS negotiation failed"))?;
            Ok(Box::new(tls))
        }
        #[cfg(test)]
        MailTransportSecurity::PlaintextForTests => {
            read_greeting(&mut tcp, config.max_message_bytes)?;
            Ok(Box::new(tcp))
        }
    }
}

fn connect_tcp(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, SyncError> {
    let mut addrs = (host, port)
        .to_socket_addrs()
        .map_err(|_| SyncError::transient("IMAP connection failed"))?;
    for addr in addrs.by_ref() {
        let Ok(stream) = TcpStream::connect_timeout(&addr, timeout) else {
            continue;
        };
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|_| SyncError::transient("IMAP connection failed"))?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|_| SyncError::transient("IMAP connection failed"))?;
        return Ok(stream);
    }

    Err(SyncError::transient("IMAP connection failed"))
}

fn read_greeting(
    stream: &mut dyn ImapProtocolStream,
    max_literal_bytes: usize,
) -> Result<(), SyncError> {
    let greeting =
        read_response(stream, max_literal_bytes).map_err(ImapReadError::into_sync_error)?;
    if greeting.line.starts_with("* OK") || greeting.line.starts_with("* PREAUTH") {
        Ok(())
    } else {
        Err(SyncError::InvalidRemoteState {
            reason: "IMAP greeting was not accepted",
        })
    }
}

fn read_until_tag_raw(
    stream: &mut dyn ImapProtocolStream,
    tag: &str,
    timeout: Duration,
    max_literal_bytes: usize,
) -> Result<Vec<ImapResponse>, SyncError> {
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|_| SyncError::transient("IMAP network operation failed"))?;
    let mut responses = Vec::new();
    loop {
        let response =
            read_response(stream, max_literal_bytes).map_err(ImapReadError::into_sync_error)?;
        let tagged = response.is_tagged(tag);
        responses.push(response);
        if tagged {
            return Ok(responses);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImapResponse {
    line: String,
    literal: Option<Vec<u8>>,
    suffix: String,
}

impl ImapResponse {
    fn text(&self) -> String {
        if self.suffix.is_empty() {
            self.line.clone()
        } else {
            format!("{}{}", self.line, self.suffix)
        }
    }

    fn is_tagged(&self, tag: &str) -> bool {
        self.line
            .strip_prefix(tag)
            .is_some_and(|rest| rest.starts_with(' '))
    }

    fn is_mailbox_change(&self) -> bool {
        let upper = self.text().to_ascii_uppercase();
        upper.starts_with("* ")
            && (upper.contains(" EXISTS")
                || upper.contains(" EXPUNGE")
                || upper.contains(" RECENT")
                || upper.contains(" FETCH "))
    }
}

#[derive(Debug)]
enum ImapReadError {
    TimedOut,
    ConnectionLost,
    Io,
    LineTooLong,
    LiteralTooLarge,
}

impl ImapReadError {
    fn into_sync_error(self) -> SyncError {
        match self {
            Self::TimedOut => SyncError::transient("IMAP operation timed out"),
            Self::ConnectionLost | Self::Io => {
                SyncError::transient("IMAP network operation failed")
            }
            Self::LineTooLong | Self::LiteralTooLarge => SyncError::InvalidRemoteState {
                reason: "IMAP protocol response was invalid",
            },
        }
    }
}

fn is_untagged_fetch(text: &str) -> bool {
    text.starts_with("* ") && text.to_ascii_uppercase().contains(" FETCH ")
}

fn read_response(
    stream: &mut dyn ImapProtocolStream,
    max_literal_bytes: usize,
) -> Result<ImapResponse, ImapReadError> {
    let line = read_protocol_line(stream)?;
    let Some(literal_len) = parse_literal_marker(&line) else {
        return Ok(ImapResponse {
            line,
            literal: None,
            suffix: String::new(),
        });
    };
    if literal_len > max_literal_bytes {
        return Err(ImapReadError::LiteralTooLarge);
    }

    let mut literal = vec![0_u8; literal_len];
    read_exact_bytes(stream, &mut literal)?;
    let suffix = read_protocol_line(stream)?;
    Ok(ImapResponse {
        line,
        literal: Some(literal),
        suffix,
    })
}

fn read_protocol_line(stream: &mut dyn ImapProtocolStream) -> Result<String, ImapReadError> {
    let mut bytes = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => return Err(ImapReadError::ConnectionLost),
            Ok(_) => {
                bytes.push(byte[0]);
                if bytes.len() > MAX_PROTOCOL_LINE_BYTES {
                    return Err(ImapReadError::LineTooLong);
                }
                if byte[0] == b'\n' {
                    return Ok(String::from_utf8_lossy(&bytes)
                        .trim_end_matches(['\r', '\n'])
                        .to_string());
                }
            }
            Err(error) if is_timeout(&error) => return Err(ImapReadError::TimedOut),
            Err(_) => return Err(ImapReadError::Io),
        }
    }
}

fn read_exact_bytes(
    stream: &mut dyn ImapProtocolStream,
    bytes: &mut [u8],
) -> Result<(), ImapReadError> {
    let mut offset = 0;
    while offset < bytes.len() {
        match stream.read(&mut bytes[offset..]) {
            Ok(0) => return Err(ImapReadError::ConnectionLost),
            Ok(read) => offset += read,
            Err(error) if is_timeout(&error) => return Err(ImapReadError::TimedOut),
            Err(_) => return Err(ImapReadError::Io),
        }
    }
    Ok(())
}

fn write_protocol_line(stream: &mut dyn ImapProtocolStream, line: &str) -> Result<(), SyncError> {
    stream
        .write_all(line.as_bytes())
        .and_then(|_| stream.write_all(b"\r\n"))
        .and_then(|_| stream.flush())
        .map_err(|_| SyncError::transient("IMAP network operation failed"))
}

fn is_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

fn parse_literal_marker(line: &str) -> Option<usize> {
    let trimmed = line.trim_end();
    if !trimmed.ends_with('}') {
        return None;
    }
    let start = trimmed.rfind('{')?;
    let marker = &trimmed[start + 1..trimmed.len() - 1];
    let marker = marker.strip_suffix('+').unwrap_or(marker);
    if marker.is_empty() || !marker.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    marker.parse().ok()
}

fn require_tagged_ok(responses: &[ImapResponse], rejected: SyncError) -> Result<(), SyncError> {
    let Some(last) = responses.last() else {
        return Err(SyncError::InvalidRemoteState {
            reason: "IMAP command returned no tagged response",
        });
    };
    let mut parts = last.line.split_whitespace();
    let _tag = parts.next();
    match parts.next() {
        Some(status) if status.eq_ignore_ascii_case("OK") => Ok(()),
        Some(status) if status.eq_ignore_ascii_case("NO") || status.eq_ignore_ascii_case("BAD") => {
            Err(rejected)
        }
        _ => Err(SyncError::InvalidRemoteState {
            reason: "IMAP tagged response was invalid",
        }),
    }
}

fn quote_imap_string(value: &str) -> Result<String, SyncError> {
    if value.contains(['\r', '\n', '\0']) {
        return Err(SyncError::InvalidRemoteState {
            reason: "IMAP string contained invalid control characters",
        });
    }

    let mut quoted = String::from("\"");
    for character in value.chars() {
        if matches!(character, '"' | '\\') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    Ok(quoted)
}

fn uid_sequence_set(uids: &[u64]) -> Result<String, SyncError> {
    if uids.contains(&0) {
        return Err(SyncError::InvalidRemoteState {
            reason: "IMAP UID must be non-zero",
        });
    }
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let mut ranges = Vec::new();
    let mut start = None;
    let mut previous = 0;
    for uid in sorted {
        match start {
            None => {
                start = Some(uid);
                previous = uid;
            }
            Some(range_start) if uid == previous.saturating_add(1) => {
                previous = uid;
                if range_start == 0 {
                    return Err(SyncError::InvalidRemoteState {
                        reason: "IMAP UID must be non-zero",
                    });
                }
            }
            Some(range_start) => {
                push_uid_range(&mut ranges, range_start, previous);
                start = Some(uid);
                previous = uid;
            }
        }
    }
    if let Some(range_start) = start {
        push_uid_range(&mut ranges, range_start, previous);
    }

    Ok(ranges.join(","))
}

fn push_uid_range(ranges: &mut Vec<String>, start: u64, end: u64) {
    if start == end {
        ranges.push(start.to_string());
    } else {
        ranges.push(format!("{start}:{end}"));
    }
}

fn imap_flag_list(flags: &BTreeSet<RemoteFlag>) -> String {
    let values = flags
        .iter()
        .map(RemoteFlag::as_imap_string)
        .collect::<Vec<_>>()
        .join(" ");
    format!("({values})")
}

fn parse_list_response(line: &str) -> Result<Option<RemoteFolder>, SyncError> {
    if !line.starts_with("* ") || !contains_word(line, "LIST") {
        return Ok(None);
    }
    let Some(attr_start) = line.find('(') else {
        return Err(SyncError::InvalidRemoteState {
            reason: "IMAP LIST response omitted attributes",
        });
    };
    let Some(attr_end) = find_matching_paren(line, attr_start) else {
        return Err(SyncError::InvalidRemoteState {
            reason: "IMAP LIST attributes were invalid",
        });
    };
    let attributes = line[attr_start + 1..attr_end]
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut index = attr_end + 1;
    let delimiter = parse_astring_or_nil(line, &mut index)?;
    let name = parse_astring(line, &mut index)?;

    Ok(Some(RemoteFolder::from_imap_attributes(
        name.clone(),
        name,
        None,
        delimiter,
        attributes,
    )))
}

fn parse_astring_or_nil(line: &str, index: &mut usize) -> Result<Option<String>, SyncError> {
    skip_spaces(line, index);
    if line
        .get(*index..)
        .is_some_and(|rest| rest.len() >= 3 && rest[..3].eq_ignore_ascii_case("NIL"))
    {
        *index += 3;
        return Ok(None);
    }

    parse_astring(line, index).map(Some)
}

fn parse_astring(line: &str, index: &mut usize) -> Result<String, SyncError> {
    skip_spaces(line, index);
    let bytes = line.as_bytes();
    if bytes.get(*index) == Some(&b'"') {
        *index += 1;
        let mut value = String::new();
        let mut escaped = false;
        while let Some(character) = line[*index..].chars().next() {
            *index += character.len_utf8();
            if escaped {
                value.push(character);
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                return Ok(value);
            } else {
                value.push(character);
            }
        }
        return Err(SyncError::InvalidRemoteState {
            reason: "IMAP quoted string was unterminated",
        });
    }

    let start = *index;
    while let Some(byte) = bytes.get(*index) {
        if byte.is_ascii_whitespace() {
            break;
        }
        *index += 1;
    }
    if start == *index {
        return Err(SyncError::InvalidRemoteState {
            reason: "IMAP atom was missing",
        });
    }
    Ok(line[start..*index].to_string())
}

fn skip_spaces(line: &str, index: &mut usize) {
    while line.as_bytes().get(*index) == Some(&b' ') {
        *index += 1;
    }
}

fn contains_word(line: &str, needle: &str) -> bool {
    line.split_whitespace()
        .any(|token| token.eq_ignore_ascii_case(needle))
}

fn find_matching_paren(line: &str, start: usize) -> Option<usize> {
    let mut escaped = false;
    let mut in_quote = false;
    let mut depth = 0_u32;
    for (index, character) in line.char_indices().skip_while(|(index, _)| *index < start) {
        if escaped {
            escaped = false;
            continue;
        }
        if in_quote {
            if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_quote = false;
            }
            continue;
        }
        match character {
            '"' => in_quote = true,
            '(' => depth = depth.saturating_add(1),
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_status_number(responses: &[ImapResponse], key: &str) -> Option<u64> {
    responses
        .iter()
        .find_map(|response| parse_bracket_number(&response.text(), key))
}

fn parse_bracket_number(line: &str, key: &str) -> Option<u64> {
    let upper = line.to_ascii_uppercase();
    let key_upper = key.to_ascii_uppercase();
    let position = upper.find(&key_upper)?;
    let after = line.get(position + key.len()..)?;
    after
        .trim_start()
        .split(|character: char| !character.is_ascii_digit())
        .next()
        .filter(|value| !value.is_empty())?
        .parse()
        .ok()
}

fn parse_exists_count(responses: &[ImapResponse]) -> Result<u32, SyncError> {
    for response in responses {
        let text = response.text();
        let mut parts = text.split_whitespace();
        if parts.next() == Some("*") {
            let Some(count) = parts.next() else {
                continue;
            };
            let Some(kind) = parts.next() else {
                continue;
            };
            if kind.eq_ignore_ascii_case("EXISTS") {
                return count.parse().map_err(|_| SyncError::InvalidRemoteState {
                    reason: "IMAP EXISTS count was invalid",
                });
            }
        }
    }

    Ok(0)
}

fn parse_search_uids(responses: &[ImapResponse]) -> BTreeSet<u64> {
    let mut uids = BTreeSet::new();
    for response in responses {
        let text = response.text();
        let mut parts = text.split_whitespace();
        if parts.next() == Some("*")
            && parts
                .next()
                .is_some_and(|token| token.eq_ignore_ascii_case("SEARCH"))
        {
            for token in parts {
                if let Ok(uid) = token.parse::<u64>() {
                    if uid > 0 {
                        uids.insert(uid);
                    }
                }
            }
        }
    }
    uids
}

fn parse_message_fetch_response(
    account_id: &str,
    folder_id: &str,
    response: &ImapResponse,
    uid_validity: u64,
) -> Result<RemoteMessage, SyncError> {
    let text = response.text();
    let uid = parse_fetch_uid(&text)?;
    let flags = parse_fetch_flags(&text);
    let internal_date = parse_quoted_item(&text, "INTERNALDATE");
    let declared_size = parse_number_after(&text, "RFC822.SIZE");
    let raw_message = response
        .literal
        .clone()
        .or_else(|| parse_quoted_fetch_body(&text).map(|value| value.into_bytes()))
        .ok_or(SyncError::InvalidRemoteState {
            reason: "IMAP FETCH response omitted message body",
        })?;
    let raw_message = ensure_message_has_body_separator(raw_message);

    let parsed = parse_inbound_mime(ParseInboundMimeRequest {
        account_id,
        folder_id,
        uid_validity,
        uid,
        raw_message: &raw_message,
        internal_date: internal_date.as_deref(),
    })
    .map(|mut parsed| {
        parsed.message.flags = flags.clone();
        parsed.message.size_bytes = declared_size.or(parsed.message.size_bytes);
        parsed.message
    })
    .unwrap_or_else(|_| {
        fallback_remote_message(
            uid_validity,
            uid,
            &raw_message,
            internal_date.as_deref(),
            flags.clone(),
        )
    });

    Ok(parsed)
}

fn parse_flag_updates(
    responses: &[ImapResponse],
    uid_validity: u64,
) -> Result<Vec<RemoteFlagUpdate>, SyncError> {
    responses
        .iter()
        .filter(|response| is_untagged_fetch(&response.text()))
        .map(|response| {
            let text = response.text();
            Ok(RemoteFlagUpdate {
                uid: parse_fetch_uid(&text)?,
                uid_validity,
                flags: parse_fetch_flags(&text),
            })
        })
        .collect()
}

fn parse_fetch_uid(text: &str) -> Result<u64, SyncError> {
    parse_number_after(text, "UID").ok_or(SyncError::InvalidRemoteState {
        reason: "IMAP FETCH response omitted UID",
    })
}

fn parse_number_after(text: &str, key: &str) -> Option<u64> {
    let upper = text.to_ascii_uppercase();
    let key_upper = key.to_ascii_uppercase();
    let mut search_from = 0;
    while let Some(relative) = upper.get(search_from..)?.find(&key_upper) {
        let position = search_from + relative;
        let before = position
            .checked_sub(1)
            .and_then(|index| text.as_bytes().get(index));
        let after_key = text.as_bytes().get(position + key.len());
        let before_ok = before.is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'.');
        let after_ok = after_key.is_none_or(|byte| byte.is_ascii_whitespace());
        if before_ok && after_ok {
            let after = text.get(position + key.len()..)?;
            return after
                .trim_start()
                .split(|character: char| !character.is_ascii_digit())
                .next()
                .filter(|value| !value.is_empty())?
                .parse()
                .ok();
        }
        search_from = position + key.len();
    }
    None
}

fn parse_fetch_flags(text: &str) -> BTreeSet<RemoteFlag> {
    let upper = text.to_ascii_uppercase();
    let Some(position) = upper.find("FLAGS") else {
        return BTreeSet::new();
    };
    let Some(open_relative) = text[position..].find('(') else {
        return BTreeSet::new();
    };
    let open = position + open_relative;
    let Some(close) = find_matching_paren(text, open) else {
        return BTreeSet::new();
    };

    RemoteFlag::from_imap_tokens(text[open + 1..close].split_whitespace())
}

fn parse_quoted_item(text: &str, key: &str) -> Option<String> {
    let upper = text.to_ascii_uppercase();
    let position = upper.find(&key.to_ascii_uppercase())?;
    let mut index = position + key.len();
    parse_astring(text, &mut index).ok()
}

fn parse_quoted_fetch_body(text: &str) -> Option<String> {
    for key in ["BODY[]", "BODY[HEADER]", "RFC822"] {
        if let Some(value) = parse_quoted_item(text, key) {
            return Some(value);
        }
    }
    None
}

fn ensure_message_has_body_separator(mut raw_message: Vec<u8>) -> Vec<u8> {
    if raw_message.windows(4).any(|window| window == b"\r\n\r\n")
        || raw_message.windows(2).any(|window| window == b"\n\n")
    {
        raw_message
    } else {
        raw_message.extend_from_slice(b"\r\n\r\n");
        raw_message
    }
}

fn fallback_remote_message(
    uid_validity: u64,
    uid: u64,
    raw_message: &[u8],
    internal_date: Option<&str>,
    flags: BTreeSet<RemoteFlag>,
) -> RemoteMessage {
    let headers = unfolded_headers(raw_message);
    let subject = header_value(&headers, "subject").unwrap_or_default();
    let from = header_value(&headers, "from");
    let message_id = header_value(&headers, "message-id");
    let date = header_value(&headers, "date").or_else(|| internal_date.map(str::to_string));

    RemoteMessage {
        uid,
        uid_validity,
        rfc_message_id: message_id,
        provider_message_id: None,
        in_reply_to: header_value(&headers, "in-reply-to"),
        subject,
        sender_name: None,
        sender_email: from,
        to_addresses_json: "[]".into(),
        cc_addresses_json: "[]".into(),
        bcc_addresses_json: "[]".into(),
        reply_to_addresses_json: "[]".into(),
        references_json: "[]".into(),
        date_received: date,
        internal_date: internal_date.map(str::to_string),
        flags,
        body_text: None,
        body_html: None,
        snippet: None,
        size_bytes: raw_message.len().try_into().ok(),
        has_attachments: false,
        parsed_from_raw_mime: false,
        attachment_records: Vec::new(),
        mime_parse_warnings_json: "[]".into(),
    }
}

fn unfolded_headers(raw_message: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(raw_message);
    let header_text = text
        .split_once("\r\n\r\n")
        .map(|(headers, _)| headers)
        .or_else(|| text.split_once("\n\n").map(|(headers, _)| headers))
        .unwrap_or(&text);
    let mut headers = Vec::<(String, String)>::new();
    for line in header_text.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some((_, value)) = headers.last_mut() {
                value.push(' ');
                value.push_str(line.trim());
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    headers
}

fn header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(header, _)| header == name)
        .map(|(_, value)| value.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        imap_sync::{ImapSyncEngine, NoopBackoffWaiter, SqliteSyncCache},
        schema::{apply_migrations, search_messages_fts},
    };
    use base64::{engine::general_purpose, Engine as _};
    use rusqlite::Connection;
    use std::{
        io::{BufRead, BufReader},
        net::{SocketAddr, TcpListener},
        sync::{Arc, Mutex},
        thread,
    };

    #[test]
    fn live_adapter_maps_core_imap_operations_to_sync_traits() {
        let server = ScriptedImapServer::start(|stream, commands| {
            let mut script = ImapScript::new(stream, commands);
            script.greet();
            script.expect_login_ok();
            script.expect_tagged(
                "LIST",
                &[
                    r#"* LIST (\HasNoChildren \Inbox) "/" "INBOX""#,
                    r#"* LIST (\HasNoChildren \Archive) "/" "Archive""#,
                ],
            );
            script.expect_select("INBOX", 2);
            script.expect_search("1 2");
            script.expect_search("2");
            script.expect_fetch_messages();
            script.expect_fetch_flags();
            script.expect_store_flags();
            script.expect_expunge();
            script.expect_capability(true);
            script.expect_idle_update();
            script.expect_logout();
        });
        let mut adapter = LiveImapAdapter::connect(password_config(&server)).unwrap();

        let folders = adapter.list_folders("acct_live").unwrap();
        assert_eq!(folders.len(), 2);
        assert_eq!(folders[0].remote_id, "INBOX");

        let status = adapter.select_folder("acct_live", "INBOX").unwrap();
        assert_eq!(status.uid_validity, 777);
        assert_eq!(status.uid_next, 3);
        assert_eq!(status.existing_uids, BTreeSet::from([1, 2]));
        assert_eq!(status.unread_count, 1);

        let messages = adapter
            .fetch_messages("acct_live", "INBOX", "folder_live_inbox", 777, &[1, 2])
            .unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].subject, "Hello from live IMAP");
        assert_eq!(
            messages[0].sender_email.as_deref(),
            Some("alice@example.com")
        );
        assert!(messages[0].flags.contains(&RemoteFlag::Seen));

        let flags = adapter
            .fetch_flags("acct_live", "INBOX", 777, &[1, 2])
            .unwrap();
        assert_eq!(flags.len(), 2);
        assert!(flags[1].flags.contains(&RemoteFlag::Flagged));

        let updates = adapter
            .store_flags(
                "acct_live",
                "INBOX",
                &[1],
                LiveImapStoreMode::Add,
                &BTreeSet::from([RemoteFlag::Flagged]),
            )
            .unwrap();
        assert_eq!(updates[0].uid, 1);
        assert!(updates[0].flags.contains(&RemoteFlag::Flagged));

        assert_eq!(adapter.expunge_folder("acct_live", "INBOX").unwrap(), 1);
        assert_eq!(
            adapter.idle_capability("acct_live", "INBOX").unwrap(),
            IdleCapability::Available
        );
        assert_eq!(
            adapter
                .idle_wait(
                    "acct_live",
                    "INBOX",
                    Duration::from_secs(1),
                    &SyncCancellationToken::new()
                )
                .unwrap(),
            IdleWaitOutcome::MailboxChanged
        );
        adapter.logout().unwrap();

        let commands = server.join();
        assert!(commands
            .iter()
            .any(|command| command.contains("UID FETCH 1:2")));
        assert!(commands
            .iter()
            .any(|command| command.contains("UID STORE 1 +FLAGS")));
        assert!(commands.iter().any(|command| command.ends_with("EXPUNGE")));
        assert!(commands.iter().any(|command| command.ends_with("IDLE")));
    }

    #[test]
    fn live_sync_persists_raw_mime_bodies_attachments_warnings_and_fts() {
        let server = ScriptedImapServer::start(|stream, commands| {
            let mut script = ImapScript::new(stream, commands);
            script.greet();
            script.expect_login_ok();
            script.expect_tagged("LIST", &[r#"* LIST (\HasNoChildren \Inbox) "/" "INBOX""#]);
            script.expect_select("INBOX", 1);
            script.expect_search("1");
            script.expect_search("");
            let line = script.read_command();
            assert!(line.contains("UID FETCH 1"));
            let tag = line.split_whitespace().next().unwrap().to_string();
            let raw = raw_message_with_attachment();
            script.write_literal_fetch(1, "\\Seen", &raw);
            script.write(&format!("{tag} OK FETCH completed\r\n"));
        });
        let mut adapter = LiveImapAdapter::connect(password_config(&server)).unwrap();
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations(&mut connection).unwrap();
        seed_live_account(&connection);
        let mut cache = SqliteSyncCache::new(&connection);

        ImapSyncEngine::default()
            .sync_account(
                &mut adapter,
                &mut cache,
                "acct_live",
                &SyncCancellationToken::new(),
                &mut NoopBackoffWaiter,
            )
            .unwrap();

        let message: (String, Option<String>, i64, String) = connection
            .query_row(
                "SELECT id, body_text, has_attachments, mime_parse_warnings_json
                 FROM messages
                 WHERE account_id = 'acct_live'
                   AND imap_uid = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(message.1.as_deref(), Some("Live MIME searchable body"));
        assert_eq!(message.2, 1);
        assert!(message.3.contains("attachment_not_storage_safe"));

        let attachment: (String, String, i64, Option<String>) = connection
            .query_row(
                "SELECT message_id, display_filename, size_bytes, storage_relative_path
                 FROM attachments
                 WHERE account_id = 'acct_live'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(attachment.0, message.0);
        assert_eq!(attachment.1, "unsafe.bin");
        assert!(attachment.2 > 0);
        assert_eq!(attachment.3, None);

        let search = search_messages_fts(&connection, "searchable", 10).unwrap();
        assert_eq!(search.len(), 1);
        assert_eq!(search[0].subject, "Live MIME Persistence");

        drop(adapter);
        server.join();
    }

    #[test]
    fn xoauth2_uses_backend_sasl_response_and_redacts_failure() {
        let server = ScriptedImapServer::start(|stream, commands| {
            let mut script = ImapScript::new(stream, commands);
            script.greet();
            let line = script.read_command();
            assert!(line.ends_with("AUTHENTICATE XOAUTH2"));
            script.write("+ \r\n");
            let response = script.read_command();
            let decoded = String::from_utf8(general_purpose::STANDARD.decode(response).unwrap())
                .expect("xoauth2 is utf8");
            assert_eq!(
                decoded,
                "user=person@example.com\x01auth=Bearer access-token\x01\x01"
            );
            let tag = line.split_whitespace().next().unwrap();
            script.write(&format!("{tag} NO invalid credentials\r\n"));
        });

        let error = LiveImapAdapter::connect(
            LiveImapConfig::new(
                "acct_oauth",
                "127.0.0.1",
                server.addr().port(),
                MailTransportSecurity::PlaintextForTests,
                LiveImapAuth::XOAuth2 {
                    username: "person@example.com".into(),
                    access_token: SecretString::new("access-token"),
                },
            )
            .with_timeout(Duration::from_secs(3)),
        )
        .expect_err("auth is rejected");

        assert_eq!(
            error,
            SyncError::PermanentServer {
                reason: "IMAP authentication was rejected".into()
            }
        );
        assert!(!error.to_string().contains("access-token"));
        server.join();
    }

    #[test]
    fn idle_wait_times_out_and_cancellation_stops_without_polling() {
        let server = ScriptedImapServer::start(|stream, commands| {
            let mut script = ImapScript::new(stream, commands);
            script.greet();
            script.expect_login_ok();
            script.expect_select("INBOX", 0);
            script.expect_search("");
            script.expect_search("");
            let line = script.read_command();
            assert!(line.ends_with("IDLE"));
            let tag = line.split_whitespace().next().unwrap().to_string();
            script.write("+ idling\r\n");
            let done = script.read_command();
            assert_eq!(done, "DONE");
            script.write(&format!("{tag} OK IDLE completed\r\n"));
            script.expect_logout();
        });
        let mut adapter = LiveImapAdapter::connect(password_config(&server)).unwrap();
        let outcome = adapter
            .idle_wait(
                "acct_live",
                "INBOX",
                Duration::from_millis(50),
                &SyncCancellationToken::new(),
            )
            .unwrap();
        assert_eq!(outcome, IdleWaitOutcome::Timeout);
        adapter.logout().unwrap();
        server.join();

        let server = ScriptedImapServer::start(|stream, commands| {
            let mut script = ImapScript::new(stream, commands);
            script.greet();
            script.expect_login_ok();
        });
        let mut adapter = LiveImapAdapter::connect(password_config(&server)).unwrap();
        let cancellation = SyncCancellationToken::new();
        cancellation.cancel();
        assert_eq!(
            adapter
                .idle_wait("acct_live", "INBOX", Duration::from_secs(1), &cancellation)
                .expect_err("cancelled before select"),
            SyncError::Cancelled
        );
        server.join();
    }

    #[test]
    fn reconnect_reauthenticates_after_network_drop() {
        let server = ScriptedImapServer::start_with_listener(|listener, commands| {
            let (first, _) = listener.accept().expect("first accept");
            {
                let mut script = ImapScript::new(first, Arc::clone(&commands));
                script.greet();
                script.expect_login_ok();
            }

            let (second, _) = listener.accept().expect("second accept");
            let mut script = ImapScript::new(second, commands);
            script.greet();
            script.expect_login_ok();
            script.expect_tagged("LIST", &[r#"* LIST (\HasNoChildren \Inbox) "/" "INBOX""#]);
            script.expect_logout();
        });
        let mut adapter = LiveImapAdapter::connect(password_config(&server)).unwrap();
        assert!(matches!(
            adapter
                .list_folders("acct_live")
                .expect_err("first socket dropped"),
            SyncError::TransientServer { .. }
        ));

        adapter.reconnect("acct_live").unwrap();
        let folders = adapter.list_folders("acct_live").unwrap();
        assert_eq!(folders[0].remote_id, "INBOX");
        adapter.logout().unwrap();
        server.join();
    }

    #[test]
    fn parser_rejects_missing_fetch_uid_and_handles_quoted_list_names() {
        let folder = parse_list_response(r#"* LIST (\HasNoChildren \Sent) "." "Sent \"Items\"""#)
            .unwrap()
            .unwrap();
        assert_eq!(folder.remote_id, "Sent \"Items\"");
        assert_eq!(folder.delimiter.as_deref(), Some("."));

        let response = ImapResponse {
            line: "* 1 FETCH (FLAGS (\\Seen) BODY[] {10}".into(),
            literal: Some(b"Subject: x".to_vec()),
            suffix: ")".into(),
        };
        assert_eq!(
            parse_message_fetch_response("acct_live", "folder_live_inbox", &response, 1)
                .expect_err("uid is required"),
            SyncError::InvalidRemoteState {
                reason: "IMAP FETCH response omitted UID"
            }
        );
    }

    fn password_config(server: &ScriptedImapServer) -> LiveImapConfig {
        LiveImapConfig::new(
            "acct_live",
            "127.0.0.1",
            server.addr().port(),
            MailTransportSecurity::PlaintextForTests,
            LiveImapAuth::Password {
                username: "person@example.com".into(),
                secret: SecretString::new("password"),
            },
        )
        .with_timeout(Duration::from_secs(3))
    }

    fn raw_message(uid: u64, subject: &str) -> Vec<u8> {
        format!(
            "Message-ID: <uid-{uid}@example.com>\r\nSubject: {subject}\r\nFrom: Alice <alice@example.com>\r\nTo: Bob <bob@example.com>\r\nDate: Fri, 22 May 2026 12:00:00 +0000\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nBody for UID {uid}"
        )
        .into_bytes()
    }

    fn raw_message_with_attachment() -> Vec<u8> {
        b"Message-ID: <live-persist@example.com>\r\nSubject: Live MIME Persistence\r\nFrom: Alice <alice@example.com>\r\nTo: Bob <bob@example.com>\r\nDate: Fri, 22 May 2026 12:00:00 +0000\r\nContent-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n--mix\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nLive MIME searchable body\r\n--mix\r\nContent-Type: application/x-msdownload; name=\"../unsafe.bin\"\r\nContent-Disposition: attachment; filename=\"../unsafe.bin\"\r\n\r\nunsupported bytes!\r\n--mix--\r\n".to_vec()
    }

    fn seed_live_account(connection: &Connection) {
        connection
            .execute(
                "INSERT INTO accounts (
                   id,
                   provider_kind,
                   display_email,
                   username,
                   auth_kind,
                   credential_ref,
                   imap_host,
                   imap_port,
                   imap_security,
                   smtp_host,
                   smtp_port,
                   smtp_security
                 )
                 VALUES (
                   'acct_live',
                   'generic_imap_smtp',
                   'live@example.com',
                   'live@example.com',
                   'app_password',
                   'komorebi:test:live',
                   '127.0.0.1',
                   993,
                   'tls',
                   'smtp.example.com',
                   465,
                   'tls'
                 )",
                [],
            )
            .unwrap();
    }

    struct ScriptedImapServer {
        addr: SocketAddr,
        commands: Arc<Mutex<Vec<String>>>,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl ScriptedImapServer {
        fn start<F>(handler: F) -> Self
        where
            F: FnOnce(TcpStream, Arc<Mutex<Vec<String>>>) + Send + 'static,
        {
            Self::start_with_listener(move |listener, commands| {
                let (stream, _) = listener.accept().expect("accept imap");
                handler(stream, commands);
            })
        }

        fn start_with_listener<F>(handler: F) -> Self
        where
            F: FnOnce(TcpListener, Arc<Mutex<Vec<String>>>) + Send + 'static,
        {
            let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind imap");
            let addr = listener.local_addr().expect("imap addr");
            let commands = Arc::new(Mutex::new(Vec::new()));
            let thread_commands = Arc::clone(&commands);
            let handle = thread::spawn(move || {
                handler(listener, thread_commands);
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

        fn join(mut self) -> Vec<String> {
            if let Some(handle) = self.handle.take() {
                handle.join().unwrap();
            }
            self.commands.lock().expect("commands").clone()
        }
    }

    struct ImapScript {
        stream: TcpStream,
        reader: BufReader<TcpStream>,
        commands: Arc<Mutex<Vec<String>>>,
    }

    impl ImapScript {
        fn new(stream: TcpStream, commands: Arc<Mutex<Vec<String>>>) -> Self {
            let reader = BufReader::new(stream.try_clone().expect("clone stream"));
            Self {
                stream,
                reader,
                commands,
            }
        }

        fn greet(&mut self) {
            self.write("* OK local IMAP fixture ready\r\n");
        }

        fn expect_login_ok(&mut self) {
            let line = self.read_command();
            assert!(line.contains(" LOGIN "));
            let tag = line.split_whitespace().next().unwrap().to_string();
            self.write(&format!("{tag} OK LOGIN completed\r\n"));
        }

        fn expect_tagged(&mut self, command: &str, untagged: &[&str]) {
            let line = self.read_command();
            assert!(line.contains(command), "expected {command}, got {line}");
            let tag = line.split_whitespace().next().unwrap().to_string();
            for response in untagged {
                self.write(response);
                self.write("\r\n");
            }
            self.write(&format!("{tag} OK {command} completed\r\n"));
        }

        fn expect_select(&mut self, folder: &str, exists: u32) {
            let line = self.read_command();
            assert!(line.contains(" SELECT "));
            assert!(line.contains(folder));
            let tag = line.split_whitespace().next().unwrap().to_string();
            self.write("* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n");
            self.write(&format!("* {exists} EXISTS\r\n"));
            self.write("* OK [UIDVALIDITY 777] UIDs valid\r\n");
            self.write("* OK [UIDNEXT 3] Predicted next UID\r\n");
            self.write(&format!("{tag} OK SELECT completed\r\n"));
        }

        fn expect_search(&mut self, uids: &str) {
            let line = self.read_command();
            assert!(line.contains("UID SEARCH"));
            let tag = line.split_whitespace().next().unwrap().to_string();
            self.write(&format!("* SEARCH {uids}\r\n"));
            self.write(&format!("{tag} OK SEARCH completed\r\n"));
        }

        fn expect_fetch_messages(&mut self) {
            let line = self.read_command();
            assert!(line.contains("UID FETCH 1:2"));
            let tag = line.split_whitespace().next().unwrap().to_string();
            let first = raw_message(1, "Hello from live IMAP");
            let second = raw_message(2, "Flagged follow-up");
            self.write_literal_fetch(1, "\\Seen", &first);
            self.write_literal_fetch(2, "\\Flagged", &second);
            self.write(&format!("{tag} OK FETCH completed\r\n"));
        }

        fn write_literal_fetch(&mut self, uid: u64, flags: &str, raw: &[u8]) {
            self.write(&format!(
                "* {uid} FETCH (UID {uid} FLAGS ({flags}) INTERNALDATE \"22-May-2026 12:00:00 +0000\" RFC822.SIZE {} BODY[] {{{}}}\r\n",
                raw.len(),
                raw.len()
            ));
            self.stream.write_all(raw).expect("write literal");
            self.write(")\r\n");
        }

        fn expect_fetch_flags(&mut self) {
            let line = self.read_command();
            assert!(line.contains("UID FETCH 1:2"));
            let tag = line.split_whitespace().next().unwrap().to_string();
            self.write("* 1 FETCH (UID 1 FLAGS (\\Seen))\r\n");
            self.write("* 2 FETCH (UID 2 FLAGS (\\Seen \\Flagged))\r\n");
            self.write(&format!("{tag} OK FETCH completed\r\n"));
        }

        fn expect_store_flags(&mut self) {
            let line = self.read_command();
            assert!(line.contains("UID STORE 1 +FLAGS (\\Flagged)"));
            let tag = line.split_whitespace().next().unwrap().to_string();
            self.write("* 1 FETCH (UID 1 FLAGS (\\Seen \\Flagged))\r\n");
            self.write(&format!("{tag} OK STORE completed\r\n"));
        }

        fn expect_expunge(&mut self) {
            let line = self.read_command();
            assert!(line.ends_with("EXPUNGE"));
            let tag = line.split_whitespace().next().unwrap().to_string();
            self.write("* 1 EXPUNGE\r\n");
            self.write(&format!("{tag} OK EXPUNGE completed\r\n"));
        }

        fn expect_capability(&mut self, idle: bool) {
            let line = self.read_command();
            assert!(line.ends_with("CAPABILITY"));
            let tag = line.split_whitespace().next().unwrap().to_string();
            let idle = if idle { " IDLE" } else { "" };
            self.write(&format!("* CAPABILITY IMAP4rev1{idle}\r\n"));
            self.write(&format!("{tag} OK CAPABILITY completed\r\n"));
        }

        fn expect_idle_update(&mut self) {
            let line = self.read_command();
            assert!(line.ends_with("IDLE"));
            let tag = line.split_whitespace().next().unwrap().to_string();
            self.write("+ idling\r\n");
            self.write("* 3 EXISTS\r\n");
            let done = self.read_command();
            assert_eq!(done, "DONE");
            self.write(&format!("{tag} OK IDLE completed\r\n"));
        }

        fn expect_logout(&mut self) {
            let line = self.read_command();
            assert!(line.ends_with("LOGOUT"));
            let tag = line.split_whitespace().next().unwrap().to_string();
            self.write("* BYE logging out\r\n");
            self.write(&format!("{tag} OK LOGOUT completed\r\n"));
        }

        fn read_command(&mut self) -> String {
            let mut line = String::new();
            self.reader.read_line(&mut line).expect("read command");
            let line = line.trim_end_matches(['\r', '\n']).to_string();
            self.commands.lock().expect("commands").push(line.clone());
            line
        }

        fn write(&mut self, value: &str) {
            self.stream
                .write_all(value.as_bytes())
                .and_then(|_| self.stream.flush())
                .expect("write response");
        }
    }
}
