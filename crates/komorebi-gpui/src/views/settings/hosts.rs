//! What the account form's server fields may hold (the Slint app's
//! `src/app/settings.rs` `validate_host`, `parse_port`, `host_for_display`,
//! `port_for_security`, and `src/app/text.rs` `FormError`): checked and
//! normalised the same way, so an account is the same in both apps.

use komorebi::providers::{Security, Server};

/// A field of the account form that can't be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proto {
    Imap,
    Smtp,
}

impl Proto {
    /// The protocol's name in a sentence (not translated).
    pub fn name(self) -> &'static str {
        match self {
            Proto::Imap => "IMAP",
            Proto::Smtp => "SMTP",
        }
    }
}

/// Why the account form can't be used as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormError {
    /// The email address has no `@`.
    Email,
    /// No provider was chosen (a new account).
    Provider,
    /// A server name is missing.
    ServerNames,
    /// A server name holds more than the name.
    ServerNameOnly,
    /// An IPv6 zone identifier (`%eth0`).
    ZoneId,
    /// A non-ASCII server name that can't be written in `xn--` form.
    AsciiName,
    Port(Proto),
    Username(Proto),
}

/// What a server name field may hold, checked and normalised: the name
/// (or IP address) only, no scheme, port, path or user. Trimmed, one
/// trailing dot dropped, lower case, an IPv6 literal unbracketed (see
/// [`host_for_display`]) and a Unicode name in its `xn--` form.
pub fn validate_host(raw: &str) -> Result<String, FormError> {
    let name_only = || FormError::ServerNameOnly;
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(FormError::ServerNames);
    }
    if raw.chars().any(|c| c.is_whitespace() || c.is_control())
        || raw.contains(['/', '\\', '@', '?', '#'])
    {
        return Err(name_only());
    }
    // An IPv6 literal, with or without brackets.
    let inner = match raw.strip_prefix('[') {
        Some(rest) => rest.strip_suffix(']').ok_or_else(name_only)?,
        None => raw,
    };
    if inner.contains(':') {
        if inner.contains('%') {
            return Err(FormError::ZoneId);
        }
        return inner
            .parse::<std::net::Ipv6Addr>()
            .map(|ip| ip.to_string())
            .map_err(|_| name_only());
    }
    if raw.starts_with('[') {
        return Err(name_only());
    }
    // A name: one trailing dot is the root, not part of the name.
    let name = raw.strip_suffix('.').unwrap_or(raw);
    if name.is_empty() || name.ends_with('.') {
        return Err(name_only());
    }
    if name.parse::<std::net::Ipv4Addr>().is_ok() {
        return Ok(name.to_string());
    }
    let ascii = if name.is_ascii() {
        name.to_ascii_lowercase()
    } else {
        idna::domain_to_ascii(name).map_err(|_| FormError::AsciiName)?
    };
    let label_ok = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    };
    if ascii.len() > 253 || !ascii.split('.').all(label_ok) {
        return Err(name_only());
    }
    Ok(ascii)
}

/// A port as typed in the form: 1 to 65535.
pub fn parse_port(raw: &str) -> Option<u16> {
    raw.trim().parse::<u16>().ok().filter(|p| *p != 0)
}

/// A stored server name as the form shows it: an IPv6 address in brackets.
pub fn host_for_display(host: &str) -> String {
    crate::text::host(host)
}

/// The default port of a server (IMAP when `imap`, else SMTP) for `security`.
pub fn default_port(imap: bool, security: Security) -> &'static str {
    match (imap, security) {
        (true, Security::Tls) => "993",
        (true, Security::StartTls) => "143",
        (false, Security::Tls) => "465",
        (false, Security::StartTls) => "587",
    }
}

/// The port field after the security changed from `old` to `new`: the new
/// default if the field was empty or still held the old default, else what
/// the user typed (a custom port such as 1143 or 2525 survives).
pub fn port_for_security(imap: bool, old: Security, new: Security, port: &str) -> String {
    let port = port.trim();
    if port.is_empty() || port == default_port(imap, old) {
        default_port(imap, new).into()
    } else {
        port.into()
    }
}

/// The server a typed host, port and security make, if they are valid.
pub fn typed_server(host: &str, port: &str, starttls: bool) -> Option<Server> {
    Some(Server {
        host: validate_host(host.trim()).ok()?,
        port: parse_port(port)?,
        security: if starttls {
            Security::StartTls
        } else {
            Security::Tls
        },
    })
}

/// `host:port` as the user would type it.
pub fn address(server: &Server) -> String {
    format!("{}:{}", host_for_display(&server.host), server.port)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ports `server_names_are_checked_and_normalised` (src/app/settings.rs).
    #[test]
    fn server_names_are_checked_and_normalised() {
        let ok =
            |raw: &str, want: &str| assert_eq!(validate_host(raw).as_deref(), Ok(want), "{raw}");
        ok("mail.example.com", "mail.example.com");
        ok("  Mail.Example.COM  ", "mail.example.com");
        ok("mail.example.com.", "mail.example.com");
        ok("localhost", "localhost");
        ok("intranet", "intranet");
        ok("mail_1.example.com", "mail_1.example.com");
        ok("192.168.1.10", "192.168.1.10");
        ok("127.0.0.1", "127.0.0.1");
        ok("::1", "::1");
        ok("[::1]", "::1");
        ok("[2001:DB8::1]", "2001:db8::1");
        ok("2001:db8::1", "2001:db8::1");
        ok("bücher.example", "xn--bcher-kva.example");

        let name_only = FormError::ServerNameOnly;
        for bad in [
            "imap://mail.example.com",
            "https://mail.example.com/",
            "mail.example.com/path",
            "mail.example.com\\x",
            "user@mail.example.com",
            "mail.example.com:993",
            "mail example.com",
            "mail\n.example.com",
            "mail..example.com",
            ".mail.example.com",
            "mail.example.com..",
            "-",
            "[::1",
            "::1]",
            "[mail.example.com]",
            "[1:2:3]",
            "mail!.example.com",
            "mail.example.com?x=1",
            "a b",
            ".",
        ] {
            assert_eq!(validate_host(bad), Err(name_only), "{bad:?}");
        }
        assert_eq!(validate_host(""), Err(FormError::ServerNames));
        assert!(validate_host("   ").is_err());
        assert_eq!(validate_host("fe80::1%eth0"), Err(FormError::ZoneId));
        assert_eq!(validate_host("[fe80::1%25eth0]"), Err(FormError::ZoneId));
        assert!(validate_host(&format!("{}.com", "a".repeat(64))).is_err());
        assert!(validate_host(&"a.".repeat(130)).is_err());
        // Bracketed for display only.
        assert_eq!(host_for_display("::1"), "[::1]");
        assert_eq!(host_for_display("mail.example.com"), "mail.example.com");
        assert_eq!(host_for_display("10.0.0.1"), "10.0.0.1");
    }

    /// Ports `a_security_change_moves_only_a_default_port`.
    #[test]
    fn a_security_change_moves_only_a_default_port() {
        use Security::{StartTls, Tls};
        let p = port_for_security;
        assert_eq!(p(true, Tls, StartTls, "993"), "143");
        assert_eq!(p(true, StartTls, Tls, "143"), "993");
        assert_eq!(p(false, Tls, StartTls, "465"), "587");
        assert_eq!(p(false, StartTls, Tls, "587"), "465");
        // A custom port stays: Proton Bridge, submission on 2525 or 25.
        assert_eq!(p(true, Tls, StartTls, "1143"), "1143");
        assert_eq!(p(false, StartTls, Tls, "2525"), "2525");
        assert_eq!(p(false, StartTls, Tls, "25"), "25");
        // The other server's default is custom here.
        assert_eq!(p(true, Tls, StartTls, "465"), "465");
        assert_eq!(p(false, Tls, StartTls, "993"), "993");
        // An empty field takes the new default; whitespace is ignored.
        assert_eq!(p(true, Tls, StartTls, ""), "143");
        assert_eq!(p(true, Tls, StartTls, " 993 "), "143");
        // Same security: nothing to move.
        assert_eq!(p(true, Tls, Tls, "993"), "993");
    }
}
