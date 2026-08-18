//! Standards-based account autodiscovery.
//!
//! Discovery order, matching Thunderbird conventions:
//! 1. **Autoconfig XML** - `https://autoconfig.<domain>/mail/config-v1.1.xml`
//! 2. **DNS SRV** (RFC 6186) - `_imaps._tcp.<domain>` / `_submission._tcp.<domain>`
//! 3. **Provider presets** - known hosting stacks get suggested defaults.
//!
//! The caller can always fall back to fully manual settings.

use super::providers::preset;
use super::types::{AuthMode, ProviderKind, Security, ServerSettings};
use anyhow::{Context, Result, anyhow};
use quick_xml::Reader;
use quick_xml::events::Event;

#[derive(Debug, Clone)]
pub struct SrvRecord {
    pub priority: u16,
    pub weight: u16,
    pub port: u16,
    pub target: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiscoveredSettings {
    pub provider_kind: String,
    pub provider_label: String,
    pub source: String,
    pub email: String,
    pub domain: String,
    pub auth_mode: String,
    pub imap: Option<ServerSettings>,
    pub smtp: Option<ServerSettings>,
    pub confidence: u8,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DiscoveryDiagnostic {
    pub source: String,
    pub status: &'static str,
    pub detail: String,
}

/// Discover IMAP/SMTP settings for `email`.
///
/// `fetch` performs the autoconfig HTTP GET (injected so tests can stub it);
/// `srv` performs an RFC 6186 SRV lookup for `_<service>._tcp.<domain>`
/// (injected so tests can stub it).
pub fn discover(
    email: &str,
    fetch: &dyn Fn(&str) -> Result<String>,
    srv: &dyn Fn(&str, &str) -> Result<Vec<SrvRecord>>,
) -> Result<DiscoveredSettings> {
    let domain = email
        .rsplit_once('@')
        .map(|(_, d)| d.to_string())
        .ok_or_else(|| anyhow!("not an email address: {email}"))?;
    let mut diagnostics = Vec::new();
    let mut warnings = Vec::new();
    let mut best: Option<DiscoveredSettings> = None;

    // 1. Autoconfig XML
    let autoconfig_url =
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={email}");
    match fetch(&autoconfig_url).and_then(|body| parse_autoconfig(&body)) {
        Ok(parsed) => {
            diagnostics.push(DiscoveryDiagnostic {
                source: "autoconfig".into(),
                status: "ok",
                detail: format!("{autoconfig_url} returned valid settings"),
            });
            let settings = DiscoveredSettings {
                provider_kind: ProviderKind::Custom.as_str().into(),
                provider_label: "Autoconfig (autoconfig XML)".into(),
                source: "autoconfig".into(),
                email: email.into(),
                domain: domain.clone(),
                auth_mode: AuthMode::Password.as_str().into(),
                imap: Some(parsed.imap),
                smtp: Some(parsed.smtp),
                confidence: 90,
                warnings: vec![],
            };
            best = Some(settings);
        }
        Err(e) => diagnostics.push(DiscoveryDiagnostic {
            source: "autoconfig".into(),
            status: "warn",
            detail: format!("{autoconfig_url}: {e}"),
        }),
    }

    // 2. DNS SRV (RFC 6186)
    if let (Ok(imap_records), Ok(smtp_records)) =
        (srv(&domain, "_imaps"), srv(&domain, "_submission"))
    {
        let imap = pick_srv(&imap_records, 993);
        let smtp = pick_srv(&smtp_records, 587);
        if let (Some(imap), Some(smtp)) = (imap, smtp) {
            diagnostics.push(DiscoveryDiagnostic {
                source: "srv".into(),
                status: "ok",
                detail: format!(
                    "RFC 6186 SRV: imap {}:{} (pri {}) / submission {}:{} (pri {})",
                    imap.target, imap.port, imap.priority, smtp.target, smtp.port, smtp.priority
                ),
            });
            let settings = DiscoveredSettings {
                provider_kind: ProviderKind::Custom.as_str().into(),
                provider_label: "DNS SRV (RFC 6186)".into(),
                source: "srv".into(),
                email: email.into(),
                domain: domain.clone(),
                auth_mode: AuthMode::Password.as_str().into(),
                imap: Some(ServerSettings {
                    host: imap.target.clone(),
                    port: imap.port,
                    security: Security::Tls,
                }),
                smtp: Some(ServerSettings {
                    host: smtp.target.clone(),
                    port: smtp.port,
                    security: Security::StartTls,
                }),
                confidence: 85,
                warnings: vec![],
            };
            if best.is_none() || settings.confidence > best.as_ref().unwrap().confidence {
                best = Some(settings);
            }
        } else {
            diagnostics.push(DiscoveryDiagnostic {
                source: "srv".into(),
                status: "info",
                detail: "no usable _imaps/_submission SRV records".into(),
            });
        }
    } else {
        diagnostics.push(DiscoveryDiagnostic {
            source: "srv".into(),
            status: "warn",
            detail: "SRV lookup failed (no SRV records or DNS unavailable)".into(),
        });
    }

    // 3. Provider preset for known hosting stacks.
    let known_kinds = [
        ProviderKind::Mailcow,
        ProviderKind::Zimbra,
        ProviderKind::Posteio,
        ProviderKind::Gmail,
        ProviderKind::Microsoft,
    ];
    let kind = known_kinds
        .into_iter()
        .find(|k| domain_known(k, &domain))
        .unwrap_or(ProviderKind::Custom);
    if kind != ProviderKind::Custom {
        if let Some(p) = preset(kind) {
            diagnostics.push(DiscoveryDiagnostic {
                source: "preset".into(),
                status: "info",
                detail: format!("known hosting stack for {domain}: {}", p.label),
            });
            let candidate = DiscoveredSettings {
                provider_kind: kind.as_str().into(),
                provider_label: p.label.into(),
                source: "preset".into(),
                email: email.into(),
                domain: domain.clone(),
                auth_mode: p.auth_mode.as_str().into(),
                imap: p.imap.clone(),
                smtp: p.smtp.clone(),
                confidence: 40,
                warnings: vec!["Server hostnames from the preset need confirmation.".into()],
            };
            if best.is_none() || candidate.confidence > best.as_ref().unwrap().confidence {
                best = Some(candidate);
            }
        }
    } else if best.is_none() {
        warnings.push(format!(
            "No automatic settings found for {domain}; enter server settings manually."
        ));
    }

    match best {
        Some(mut settings) => {
            settings.warnings.extend(warnings);
            Ok(settings)
        }
        None => Err(anyhow!(
            "no IMAP/SMTP settings discovered for {email}; please configure manually"
        )),
    }
}

fn domain_known(kind: &ProviderKind, domain: &str) -> bool {
    match kind {
        ProviderKind::Gmail => domain == "gmail.com" || domain == "googlemail.com",
        ProviderKind::Microsoft => {
            domain == "outlook.com"
                || domain == "hotmail.com"
                || domain == "live.com"
                || domain.ends_with("onmicrosoft.com")
        }
        _ => false,
    }
}

fn pick_srv(records: &[SrvRecord], default_port: u16) -> Option<SrvRecord> {
    records
        .iter()
        .min_by_key(|r| (r.priority, r.weight))
        .cloned()
        .map(|mut r| {
            if r.port == 0 {
                // RFC 6186: port 0 means "use the default".
                r.port = default_port;
            }
            r
        })
}

/// Parse a Thunderbird-style autoconfig XML document.
fn parse_autoconfig(xml: &str) -> Result<Autoconfig> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut path: Vec<String> = Vec::new();
    let mut incoming = ServerSettings {
        host: String::new(),
        port: 993,
        security: Security::Tls,
    };
    let mut outgoing = ServerSettings {
        host: String::new(),
        port: 587,
        security: Security::StartTls,
    };
    let mut current: Option<&mut ServerSettings> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                path.push(name.clone());
                match name.as_str() {
                    "incomingServer" => current = Some(&mut incoming),
                    "outgoingServer" => current = Some(&mut outgoing),
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                path.pop();
                if name == "incomingServer" || name == "outgoingServer" {
                    current = None;
                }
            }
            Ok(Event::Text(t)) => {
                let text = t.decode().unwrap_or_default().trim().to_string();
                if text.is_empty() {
                    continue;
                }
                let leaf = path.last().map(|s| s.as_str());
                if let Some(settings) = current.as_deref_mut() {
                    match leaf {
                        Some("hostname") => settings.host = text,
                        Some("port") => {
                            if let Ok(p) = text.parse() {
                                settings.port = p;
                            }
                        }
                        Some("socketType") => {
                            settings.security = match text.as_str() {
                                "SSL" => Security::Tls,
                                "STARTTLS" => Security::StartTls,
                                _ => Security::None,
                            };
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow!("invalid autoconfig XML: {e}")),
            _ => {}
        }
    }

    if incoming.host.is_empty() || outgoing.host.is_empty() {
        return Err(anyhow!("autoconfig XML missing hostname entries"));
    }
    Ok(Autoconfig {
        imap: incoming,
        smtp: outgoing,
    })
}

struct Autoconfig {
    imap: ServerSettings,
    smtp: ServerSettings,
}

/// Real SRV lookup through the system DNS resolver (hickory + tokio
/// current-thread runtime). Privacy note: queries go to the machine's
/// configured DNS servers only.
pub fn srv_records(domain: &str, service: &str) -> Result<Vec<SrvRecord>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building DNS runtime")?;
    let query = format!("_{service}._tcp.{domain}");
    rt.block_on(async move {
        use hickory_resolver::TokioResolver;
        let resolver = TokioResolver::builder_tokio()
            .context("reading system DNS config")?
            .build()
            .context("building DNS resolver")?;
        let lookup = resolver
            .srv_lookup(&query)
            .await
            .map_err(|e| anyhow!("SRV lookup {query}: {e}"))?;
        let mut out = Vec::new();
        for record in lookup.answers() {
            if let hickory_resolver::proto::rr::RData::SRV(srv) = &record.data {
                out.push(SrvRecord {
                    priority: srv.priority,
                    weight: srv.weight,
                    port: srv.port,
                    target: srv.target.to_string(),
                });
            }
        }
        Ok(out)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTOCONFIG: &str = r#"<?xml version="1.0"?>
<clientConfig version="1.1">
  <emailProvider id="example.com">
    <domain>example.com</domain>
    <displayName>Example Mail</displayName>
    <incomingServer type="imap">
      <hostname>imap.example.com</hostname>
      <port>993</port>
      <socketType>SSL</socketType>
      <authentication>password-cleartext</authentication>
      <username>%EMAILADDRESS%</username>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>smtp.example.com</hostname>
      <port>465</port>
      <socketType>SSL</socketType>
      <authentication>password-cleartext</authentication>
      <username>%EMAILADDRESS%</username>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#;

    #[test]
    fn parses_autoconfig_xml() {
        let parsed = parse_autoconfig(AUTOCONFIG).unwrap();
        assert_eq!(parsed.imap.host, "imap.example.com");
        assert_eq!(parsed.imap.port, 993);
        assert_eq!(parsed.imap.security, Security::Tls);
        assert_eq!(parsed.smtp.host, "smtp.example.com");
        assert_eq!(parsed.smtp.port, 465);
    }

    #[test]
    fn autoconfig_wins_over_srv() {
        let fetch = |url: &str| {
            assert!(url.contains("autoconfig.example.com"));
            Ok(AUTOCONFIG.to_string())
        };
        let srv = |_d: &str, s: &str| {
            Ok(vec![SrvRecord {
                priority: 10,
                weight: 5,
                port: 993,
                target: format!("{s}.example.com"),
            }])
        };
        let found = discover("me@example.com", &fetch, &srv).unwrap();
        assert_eq!(found.source, "autoconfig");
        assert_eq!(found.imap.unwrap().host, "imap.example.com");
    }

    #[test]
    fn srv_fallback_when_no_autoconfig() {
        let fetch = |_url: &str| Err(anyhow!("404"));
        let srv = |_d: &str, s: &str| {
            Ok(vec![SrvRecord {
                priority: 0,
                weight: 0,
                port: 0, // RFC 6186 default port
                target: format!("mail.{s}.example.net"),
            }])
        };
        let found = discover("me@example.net", &fetch, &srv).unwrap();
        assert_eq!(found.source, "srv");
        let imap = found.imap.unwrap();
        assert_eq!(imap.port, 993); // default port applied
    }

    #[test]
    fn preset_recognizes_gmail() {
        let fetch = |_url: &str| Err(anyhow!("404"));
        let srv = |_d: &str, _s: &str| Err(anyhow!("no records"));
        let found = discover("me@gmail.com", &fetch, &srv).unwrap();
        assert_eq!(found.provider_kind, "gmail");
        assert_eq!(found.auth_mode, "oauth");
        assert_eq!(found.source, "preset");
    }

    #[test]
    fn manual_fallback_when_nothing_found() {
        let fetch = |_url: &str| Err(anyhow!("404"));
        let srv = |_d: &str, _s: &str| Err(anyhow!("no records"));
        let err = discover("me@nowhere.invalid", &fetch, &srv).unwrap_err();
        assert!(err.to_string().contains("manually"));
    }
}
