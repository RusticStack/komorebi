//! Finding a DAV server (RFC 6764): SRV `_carddavs._tcp` or
//! `_caldavs._tcp`, then `https://<host>/.well-known/carddav` (or
//! `caldav`), whose redirect is read without sending credentials. Then,
//! authenticated and only on a user's press: the principal, its home set
//! and the collections in it. TXT `path` records are not read. Refused
//! under `KOMOREBI_OFFLINE`.

use super::{Client, DavError, xml};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    CardDav,
    CalDav,
}

impl Service {
    fn srv(self) -> &'static str {
        match self {
            Service::CardDav => "_carddavs._tcp",
            Service::CalDav => "_caldavs._tcp",
        }
    }

    fn well_known(self) -> &'static str {
        match self {
            Service::CardDav => "/.well-known/carddav",
            Service::CalDav => "/.well-known/caldav",
        }
    }
}

/// A start URL for `domain`, without credentials: the SRV target (or the
/// domain) with its well-known path, the redirect it gives if any.
pub fn well_known(service: Service, domain: &str) -> Result<String, DavError> {
    if crate::dev_var("KOMOREBI_OFFLINE").is_some() {
        return Err(DavError::Offline);
    }
    let domain = domain.trim().trim_end_matches('.');
    let host = crate::mail::discover::srv_ordered(&format!("{}.{domain}", service.srv()))
        .into_iter()
        .next()
        .map(|r| {
            let t = r.target.trim_end_matches('.').to_string();
            if r.port == 443 {
                t
            } else {
                format!("{t}:{}", r.port)
            }
        })
        .unwrap_or_else(|| domain.to_string());
    let start = format!("https://{host}{}", service.well_known());
    let client = Client::anonymous(&start)?;
    first_hop(&client, &start)
}

/// Where `url` redirects (itself when it doesn't), read with one
/// credential-free request.
pub(crate) fn first_hop(client: &Client, url: &str) -> Result<String, DavError> {
    let u = client.resolve(url)?;
    let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let (reply, location) = client.once("PROPFIND", &u, &[("Depth", "0")], &[], end)?;
    match (reply.status, location) {
        (301 | 302 | 303 | 307 | 308, Some(l)) => {
            let next = u.join(&l).map_err(|_| DavError::Protocol)?;
            if next.scheme() != "https" {
                return Err(DavError::Refused);
            }
            Ok(next.to_string())
        }
        _ => Ok(u.to_string()),
    }
}

/// The home set for `service` starting from `url` (the URL itself when it
/// is already a collection of that kind, or holds no principal).
pub fn home(client: &Client, url: &str, service: Service) -> Result<String, DavError> {
    let top = client.propfind(url, "0", xml::PROPFIND_PRINCIPAL)?;
    let first = top.items.first().cloned().unwrap_or_default();
    let principal = first.principal.clone().unwrap_or_else(|| url.to_string());
    let homes = client.propfind(&principal, "0", xml::PROPFIND_HOMES)?;
    let found = homes.items.first().and_then(|i| match service {
        Service::CardDav => i.addressbook_home.clone(),
        Service::CalDav => i.calendar_home.clone(),
    });
    Ok(found.unwrap_or_else(|| url.to_string()))
}

/// The collections of `service` in `home` (or `home` itself, when it is
/// one).
pub fn collections(
    client: &Client,
    home: &str,
    service: Service,
) -> Result<Vec<xml::Item>, DavError> {
    let m = client.propfind(home, "1", xml::PROPFIND_COLLECTIONS)?;
    Ok(m.items
        .into_iter()
        .filter(|i| match service {
            Service::CardDav => i.addressbook,
            Service::CalDav => i.calendar,
        })
        .collect())
}

/// Whether credentials may go to `url` found by lookup for an address at
/// `domain` without asking: its host is that domain or under it. Any other
/// host (an SRV target or a redirect elsewhere) is shown to the user first.
pub fn same_domain(url: &str, domain: &str) -> bool {
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    let Some(host) = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
    else {
        return false;
    };
    let host = host.trim_end_matches('.');
    !domain.is_empty()
        && (host == domain
            || host
                .strip_suffix(domain.as_str())
                .is_some_and(|rest| rest.ends_with('.')))
}

#[cfg(test)]
mod tests {
    use super::same_domain;

    #[test]
    fn credentials_only_to_the_mail_domain() {
        assert!(same_domain("https://example.com/dav/", "example.com"));
        assert!(same_domain("https://dav.Example.com./x", "example.com"));
        assert!(!same_domain("https://evilexample.com/", "example.com"));
        assert!(!same_domain("https://example.com.evil.net/", "example.com"));
        assert!(!same_domain("https://dav.other.net/", "example.com"));
        assert!(!same_domain("not a url", "example.com"));
        assert!(!same_domain("https://example.com/", ""));
    }
}
