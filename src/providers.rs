//! Built-in provider presets. Picking one fills in every server setting, so
//! adding an account needs only an address and a password.
//!
//! The table is data: each preset's servers, the email domains it owns (an
//! address there is its account), the MX host suffixes of the custom domains
//! it hosts (`mail::discover` maps a domain's MX answer to a preset, never to
//! a server named by DNS), how its accounts sign in, and a comment with the
//! official help page each value comes from (read on 2026-11-14). The
//! account setup lists them in this order, which is alphabetical.
//!
//! Komorebi signs in with a password only (IMAP LOGIN, SMTP AUTH PLAIN or
//! LOGIN; no OAuth 2 / XOAUTH2). A provider that turned password sign-in off
//! is listed as [`SignIn::NotSupported`]: shown, but it cannot be picked.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// Implicit TLS (IMAP 993, SMTP 465).
    Tls,
    /// Plain connect upgraded with STARTTLS (SMTP 587).
    StartTls,
}

impl Security {
    pub fn as_i64(self) -> i64 {
        match self {
            Self::Tls => 1,
            Self::StartTls => 0,
        }
    }
    pub fn from_i64(v: i64) -> Self {
        if v == 0 { Self::StartTls } else { Self::Tls }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub security: Security,
}

/// How a preset's accounts sign in, which is what the list's note says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignIn {
    /// The account's own password.
    Password,
    /// The account's password, or an app password once two-step sign-in is
    /// on.
    AppPasswordIfTwoStep,
    /// An app password (or app-specific / device password) only.
    AppPassword,
    /// IMAP must be turned on in the provider's web settings first.
    TurnOnImap,
    /// IMAP must be turned on and an app password made first.
    TurnOnImapAppPassword,
    /// IMAP is for paid plans, and must be turned on.
    PaidImap,
    /// Proton Mail Bridge, running on this PC, is the server: the form is
    /// filled in and stays editable (its ports and its self-signed
    /// certificate are the Bridge's), and the password is the one Bridge
    /// shows.
    Bridge,
    /// The provider allows only its own sign-in (OAuth 2), which Komorebi
    /// does not have yet: listed so users know, never picked.
    NotSupported,
}

/// A server of a preset: host, port, security.
pub type Endpoint = (&'static str, u16, Security);

pub struct Preset {
    /// A stable name for the preset: its icon file (`assets/providers`),
    /// the examples and the tests. Never shown.
    pub id: &'static str,
    /// The name the list shows (a brand: not translated).
    pub name: &'static str,
    /// The provider's own mail domains: an address there is its account.
    /// Lower case, exact (no wildcards).
    pub domains: &'static [&'static str],
    /// The MX host suffixes of custom domains the provider hosts: an MX host
    /// equal to one, or ending in `.` and one, means the domain's mail is
    /// here. Only from the provider's own custom-domain help.
    pub mx_suffixes: &'static [&'static str],
    pub imap: Endpoint,
    pub smtp: Endpoint,
    pub sign_in: SignIn,
    /// Whether a new account on this preset saves a copy of what it sends
    /// to its Sent folder. SMTP never does that by itself, so it is on
    /// unless the provider is known to save sent mail on the server (Gmail
    /// does). Postale.io is known (from live tests) not to, so it is on;
    /// elsewhere a duplicate is avoided by the Message-ID lookup of the Sent
    /// folder. Existing accounts keep their stored choice.
    pub save_sent: bool,
}

use Security::{StartTls, Tls};

/// Presets in the order of the setup's provider list (alphabetical, case
/// aside); "Add provider" (any other server) follows them in the UI.
pub const PRESETS: &[Preset] = &[
    // https://help.aol.com/articles/how-do-i-use-other-email-applications-to-send-and-receive-my-aol-mail
    // https://help.aol.com/articles/create-and-manage-app-password
    Preset {
        id: "aol",
        name: "AOL Mail",
        domains: &["aol.com", "aim.com", "aol.co.uk", "aol.de", "aol.fr"],
        mx_suffixes: &[],
        imap: ("imap.aol.com", 993, Tls),
        smtp: ("smtp.aol.com", 465, Tls),
        sign_in: SignIn::AppPassword,
        save_sent: true,
    },
    // https://www.fastmail.help/hc/en-us/articles/1500000278342-Server-names-and-ports
    // Domains: https://www.fastmail.help/hc/en-us/articles/6239365719055
    // MX: https://www.fastmail.help/hc/en-us/articles/1500000280261-Setting-up-your-domain-MX-only
    Preset {
        id: "fastmail",
        name: "Fastmail",
        domains: &[
            "fastmail.com",
            "fastmail.fm",
            "fastmail.net",
            "fastmail.org",
            "fastmail.ca",
            "fastmail.co.uk",
            "fastmail.com.au",
            "fastmail.de",
            "fastmail.es",
            "fastmail.fr",
            "fastmail.nl",
            "fastmail.se",
            "fastmail.uk",
            "fastmail.us",
        ],
        mx_suffixes: &["messagingengine.com"],
        imap: ("imap.fastmail.com", 993, Tls),
        smtp: ("smtp.fastmail.com", 465, Tls),
        sign_in: SignIn::AppPassword,
        save_sent: true,
    },
    // https://assistance.free.fr/articles/609
    Preset {
        id: "free",
        name: "Free",
        domains: &["free.fr"],
        mx_suffixes: &[],
        imap: ("imap.free.fr", 993, Tls),
        smtp: ("smtp.free.fr", 465, Tls),
        sign_in: SignIn::Password,
        save_sent: true,
    },
    // https://developers.google.com/workspace/gmail/imap/imap-smtp
    // https://support.google.com/mail/answer/7126229 (app passwords with
    // two-step verification; IMAP is always on since January 2025)
    // Gmail saves what it sends to Sent itself.
    Preset {
        id: "gmail",
        name: "Gmail",
        domains: &["gmail.com", "googlemail.com"],
        mx_suffixes: &[],
        imap: ("imap.gmail.com", 993, Tls),
        smtp: ("smtp.gmail.com", 465, Tls),
        sign_in: SignIn::AppPassword,
        save_sent: false,
    },
    // https://hilfe.gmx.net/pop-imap/imap/imap-serverdaten.html
    // https://hilfe.gmx.net/account/logindaten/app-passwort.html
    Preset {
        id: "gmx",
        name: "GMX",
        domains: &["gmx.net", "gmx.de", "gmx.at", "gmx.ch", "gmx.eu"],
        mx_suffixes: &[],
        imap: ("imap.gmx.net", 993, Tls),
        smtp: ("mail.gmx.net", 465, Tls),
        sign_in: SignIn::TurnOnImap,
        save_sent: true,
    },
    // https://support.gmx.com/pop-imap/imap/server.html
    // https://support.gmx.com/pop-imap/toggle.html
    Preset {
        id: "gmx-com",
        name: "GMX.com",
        domains: &[
            "gmx.com",
            "gmx.us",
            "gmx.co.uk",
            "gmx.fr",
            "gmx.es",
            "gmx.it",
            "gmx.ca",
        ],
        mx_suffixes: &[],
        imap: ("imap.gmx.com", 993, Tls),
        smtp: ("mail.gmx.com", 465, Tls),
        sign_in: SignIn::TurnOnImap,
        save_sent: true,
    },
    // https://knowledge.workspace.google.com/admin/sync/set-up-gmail-with-a-third-party-email-client
    // MX: https://knowledge.workspace.google.com/admin/domains/set-up-mx-records-for-google-workspace
    // (smtp.google.com; older domains aspmx.l.google.com and
    // aspmx2.googlemail.com). An app password since May 2025.
    Preset {
        id: "google-workspace",
        name: "Google Workspace",
        domains: &[],
        mx_suffixes: &["google.com", "googlemail.com"],
        imap: ("imap.gmail.com", 993, Tls),
        smtp: ("smtp.gmail.com", 465, Tls),
        sign_in: SignIn::AppPassword,
        save_sent: false,
    },
    // https://support.apple.com/en-us/102525 (servers, app-specific password)
    // Domains: https://support.apple.com/en-us/118230
    // MX: https://support.apple.com/en-us/102374 (mx01/mx02.mail.icloud.com)
    Preset {
        id: "icloud",
        name: "iCloud Mail",
        domains: &["icloud.com", "me.com", "mac.com"],
        mx_suffixes: &["mail.icloud.com"],
        imap: ("imap.mail.me.com", 993, Tls),
        smtp: ("smtp.mail.me.com", 587, StartTls),
        sign_in: SignIn::AppPassword,
        save_sent: true,
    },
    // https://www.infomaniak.com/en/support/faq/2427/sync-your-emails-across-all-your-devices
    // https://www.infomaniak.com/en/support/faq/468/understanding-ports-and-messaging-protocols
    // Free domains: https://www.infomaniak.com/en/support/faq/2904/manage-my-ksuite-passwords-ikme-etikcom-ikmailcom
    // MX: https://www.infomaniak.com/en/support/faq/2110/manage-mx-records (mta-gw.infomaniak.ch)
    Preset {
        id: "infomaniak",
        name: "Infomaniak",
        domains: &["ik.me", "etik.com", "ikmail.com"],
        mx_suffixes: &["infomaniak.ch"],
        imap: ("mail.infomaniak.com", 993, Tls),
        smtp: ("mail.infomaniak.com", 465, Tls),
        sign_in: SignIn::AppPassword,
        save_sent: true,
    },
    // https://aiuto.libero.it/articolo/mail/configurare-libero-mail-con-client-di-posta-imap-e-smtp/
    // https://aiuto.libero.it/articolo/sicurezza/password-sicura-specifica-applicazioni-client/
    Preset {
        id: "libero",
        name: "Libero Mail",
        domains: &["libero.it", "iol.it", "blu.it", "inwind.it"],
        mx_suffixes: &[],
        imap: ("imapmail.libero.it", 993, Tls),
        smtp: ("smtp.libero.it", 465, Tls),
        sign_in: SignIn::AppPasswordIfTwoStep,
        save_sent: true,
    },
    // https://support.mail.com/premium/imap/server.html
    // https://support.mail.com/pop-imap/setting-up-pop-imap.html (paid plans)
    Preset {
        id: "mailcom",
        name: "mail.com",
        domains: &[
            "mail.com",
            "email.com",
            "usa.com",
            "post.com",
            "myself.com",
            "consultant.com",
            "engineer.com",
            "dr.com",
            "accountant.com",
            "cheerful.com",
            "techie.com",
            "writeme.com",
            "iname.com",
            "europe.com",
        ],
        mx_suffixes: &[],
        imap: ("imap.mail.com", 993, Tls),
        smtp: ("smtp.mail.com", 465, Tls),
        sign_in: SignIn::PaidImap,
        save_sent: true,
    },
    // https://kb.mailbox.org/en/private/e-mail/e-mail-configuration/
    // MX: https://kb.mailbox.org/en/private/custom-domains/using-emails-with-a-custom-domain/
    Preset {
        id: "mailboxorg",
        name: "mailbox.org",
        domains: &["mailbox.org"],
        mx_suffixes: &["mailbox.org"],
        imap: ("imap.mailbox.org", 993, Tls),
        smtp: ("smtp.mailbox.org", 465, Tls),
        sign_in: SignIn::Password,
        save_sent: true,
    },
    // https://learn.microsoft.com/en-us/exchange/clients-and-mobile-in-exchange-online/deprecation-of-basic-authentication-exchange-online
    // ("Basic authentication is now disabled in all tenants", for IMAP).
    // MX: https://learn.microsoft.com/en-us/purview/how-smtp-dane-works
    // (<domain>.mail.protection.outlook.com, <domain>.o-v1.mx.microsoft).
    Preset {
        id: "microsoft365",
        name: "Microsoft 365",
        domains: &[],
        mx_suffixes: &["mail.protection.outlook.com", "mx.microsoft"],
        imap: ("outlook.office365.com", 993, Tls),
        smtp: ("smtp.office365.com", 587, StartTls),
        sign_in: SignIn::NotSupported,
        save_sent: false,
    },
    // https://assistance.orange.fr/mobile-tablette/tous-les-mobiles-et-tablettes/installer-et-utiliser/utiliser-internet-mail-et-cloud/mail/l-application-mail-de-votre-telephone/parametrer-et-configurer/orange-dive-73-configurer-l-acces-a-votre-boite-mail-orange-manuellement_255453-797106
    Preset {
        id: "orange",
        name: "Orange",
        domains: &["orange.fr", "wanadoo.fr"],
        mx_suffixes: &[],
        imap: ("imap.orange.fr", 993, Tls),
        smtp: ("smtp.orange.fr", 465, Tls),
        sign_in: SignIn::Password,
        save_sent: true,
    },
    // https://support.microsoft.com/en-us/office/pop-imap-and-smtp-settings-for-outlook-com-d088b986-291d-42b8-9564-9c414e2aa040
    // ("Outlook.com requires the use of Modern Auth / OAuth2.")
    // https://support.microsoft.com/en-us/support/known-issues/modern-authentication-methods-now-needed-to-continue-syncing-outlook-email-in-non-microsoft-email-ap
    // (Basic authentication ended on 16 September 2024.)
    Preset {
        id: "outlook",
        name: "Outlook.com",
        domains: &[
            "outlook.com",
            "hotmail.com",
            "live.com",
            "msn.com",
            "windowslive.com",
            "outlook.fr",
            "outlook.de",
            "outlook.es",
            "outlook.it",
            "outlook.pt",
            "hotmail.co.uk",
            "hotmail.fr",
            "hotmail.de",
            "hotmail.it",
            "hotmail.es",
            "hotmail.com.br",
            "live.co.uk",
            "live.fr",
            "live.de",
            "live.it",
            "live.com.pt",
        ],
        mx_suffixes: &[],
        imap: ("outlook.office365.com", 993, Tls),
        smtp: ("smtp-mail.outlook.com", 587, StartTls),
        sign_in: SignIn::NotSupported,
        save_sent: false,
    },
    // https://postale.io/client/ (servers), https://postale.io/configure
    // (MX: mail.postale.io). Postale.io hosts custom domains only.
    Preset {
        id: "postale",
        name: "Postale.io",
        domains: &["postale.io"],
        mx_suffixes: &["postale.io"],
        imap: ("mail.postale.io", 993, Tls),
        smtp: ("mail.postale.io", 465, Tls),
        sign_in: SignIn::Password,
        save_sent: true,
    },
    // https://posteo.de/en/help/how-do-i-set-up-posteo-in-an-email-client-pop3-imap-and-smtp
    // (no custom domains: https://posteo.de/en/site/faq)
    Preset {
        id: "posteo",
        name: "Posteo",
        domains: &[
            "posteo.de",
            "posteo.net",
            "posteo.com",
            "posteo.at",
            "posteo.ch",
            "posteo.be",
            "posteo.nl",
            "posteo.lu",
            "posteo.uk",
            "posteo.ie",
            "posteo.es",
            "posteo.us",
            "posteo.ca",
        ],
        mx_suffixes: &[],
        imap: ("posteo.de", 993, Tls),
        smtp: ("posteo.de", 465, Tls),
        sign_in: SignIn::AppPassword,
        save_sent: true,
    },
    // Proton Mail Bridge: https://proton.me/support/port-already-occupied-error
    // (ports 1143 and 1025), https://proton.me/support/bridge-ssl-connection-issue
    // (127.0.0.1, STARTTLS, a self-signed certificate).
    // Domains: https://proton.me/support/what-is-difference-between-proton-domains
    // MX: https://proton.me/support/custom-domain (mail/mailsec.protonmail.ch)
    Preset {
        id: "proton",
        name: "Proton Mail",
        domains: &["proton.me", "protonmail.com", "pm.me", "protonmail.ch"],
        mx_suffixes: &["protonmail.ch"],
        imap: ("127.0.0.1", 1143, StartTls),
        smtp: ("127.0.0.1", 1025, StartTls),
        sign_in: SignIn::Bridge,
        save_sent: true,
    },
    // https://support.purelymail.com/support/solutions/articles/159000430778-server-settings-imap-smtp-and-pop3
    // MX: https://purelymail.com/docs/domainDocs (mailserver.purelymail.com).
    // Purelymail is undocumented on sent copies: the general default (on).
    Preset {
        id: "purelymail",
        name: "Purelymail",
        domains: &["purelymail.com"],
        mx_suffixes: &["purelymail.com"],
        imap: ("imap.purelymail.com", 993, Tls),
        smtp: ("smtp.purelymail.com", 465, Tls),
        sign_in: SignIn::AppPasswordIfTwoStep,
        save_sent: true,
    },
    // https://o-seznam.cz/napoveda/email/mohlo-by-se-hodit/postovni-programy-a-aplikace/
    Preset {
        id: "seznam",
        name: "Seznam Email",
        domains: &["seznam.cz", "email.cz", "post.cz", "spoluzaci.cz"],
        mx_suffixes: &[],
        imap: ("imap.seznam.cz", 993, Tls),
        smtp: ("smtp.seznam.cz", 465, Tls),
        sign_in: SignIn::Password,
        save_sent: true,
    },
    // https://hilfe.web.de/pop-imap/imap/imap-serverdaten.html (IMAP must be
    // switched on in the WEB.DE settings)
    Preset {
        id: "webde",
        name: "WEB.DE",
        domains: &["web.de"],
        mx_suffixes: &[],
        imap: ("imap.web.de", 993, Tls),
        smtp: ("smtp.web.de", 465, Tls),
        sign_in: SignIn::TurnOnImap,
        save_sent: true,
    },
    // https://help.yahoo.com/kb/SLN4075.html (servers),
    // https://help.yahoo.com/kb/SLN15241.html (app password)
    Preset {
        id: "yahoo",
        name: "Yahoo Mail",
        domains: &[
            "yahoo.com",
            "yahoo.co.uk",
            "yahoo.fr",
            "yahoo.de",
            "yahoo.it",
            "yahoo.es",
            "yahoo.ca",
            "yahoo.se",
            "yahoo.co.in",
            "yahoo.co.nz",
            "yahoo.com.au",
            "yahoo.com.br",
            "yahoo.com.mx",
            "yahoo.com.ar",
            "ymail.com",
            "rocketmail.com",
            "myyahoo.com",
        ],
        mx_suffixes: &[],
        imap: ("imap.mail.yahoo.com", 993, Tls),
        smtp: ("smtp.mail.yahoo.com", 465, Tls),
        sign_in: SignIn::AppPassword,
        save_sent: true,
    },
    // https://yandex.com/support/yandex-360/customers/mail/en/mail-clients/others
    // MX: https://yandex.com/support/yandex-360/business/admin/en/domains/dns/mx
    Preset {
        id: "yandex",
        name: "Yandex Mail",
        domains: &[
            "yandex.com",
            "yandex.ru",
            "ya.ru",
            "yandex.by",
            "yandex.kz",
            "narod.ru",
        ],
        mx_suffixes: &["yandex.net"],
        imap: ("imap.yandex.com", 993, Tls),
        smtp: ("smtp.yandex.com", 465, Tls),
        sign_in: SignIn::TurnOnImapAppPassword,
        save_sent: true,
    },
    // https://www.zoho.com/mail/help/imap-access.html,
    // https://www.zoho.com/mail/help/zoho-smtp.html (personal and free
    // organisation accounts: imap.zoho.com and smtp.zoho.com)
    Preset {
        id: "zoho",
        name: "Zoho Mail",
        domains: &["zoho.com", "zohomail.com"],
        mx_suffixes: &[],
        imap: ("imap.zoho.com", 993, Tls),
        smtp: ("smtp.zoho.com", 465, Tls),
        sign_in: SignIn::TurnOnImap,
        save_sent: true,
    },
    // Zoho's EU data centre (personal accounts): a Zoho staff answer,
    // https://help.zoho.com/portal/en/community/topic/imap-smtp-servers
    // (the help pages above list only the .com hosts).
    Preset {
        id: "zoho-eu",
        name: "Zoho Mail EU",
        domains: &["zoho.eu", "zohomail.eu"],
        mx_suffixes: &[],
        imap: ("imap.zoho.eu", 993, Tls),
        smtp: ("smtp.zoho.eu", 465, Tls),
        sign_in: SignIn::TurnOnImap,
        save_sent: true,
    },
    // The Zoho Mail help pages above: a paid organisation's (custom domain) accounts use
    // imappro.zoho.com and smtppro.zoho.com.
    // MX: https://www.zoho.com/mail/help/adminconsole/configure-email-delivery.html
    // (mx.zoho.com, mx2.zoho.com, mx3.zoho.com)
    Preset {
        id: "zoho-business",
        name: "Zoho Mail for business",
        domains: &[],
        mx_suffixes: &["zoho.com"],
        imap: ("imappro.zoho.com", 993, Tls),
        smtp: ("smtppro.zoho.com", 465, Tls),
        sign_in: SignIn::TurnOnImap,
        save_sent: true,
    },
];

/// The lower case domain of an address (case, spaces and a trailing dot
/// aside), if it has one.
fn domain_of(email: &str) -> Option<String> {
    let (_, domain) = email.trim().rsplit_once('@')?;
    let domain = domain.trim_end_matches('.').to_ascii_lowercase();
    (!domain.is_empty()).then_some(domain)
}

/// The preset an address's domain names: an address at gmail.com or
/// postale.io is that provider's (case and spaces aside). Any other domain
/// (a custom one hosted anywhere) names none: its MX records may
/// (`mail::discover`), or the user picks.
pub fn guess(email: &str) -> Option<usize> {
    let domain = domain_of(email)?;
    PRESETS
        .iter()
        .position(|p| p.domains.contains(&domain.as_str()))
}

/// Whether `email` is on preset `index`'s own domain.
pub fn on_own_domain(index: usize, email: &str) -> bool {
    guess(email) == Some(index)
}

/// The preset that hosts mail whose MX host is `host`: the first preset
/// with a suffix that `host` is, or ends in after a dot (case and a trailing
/// dot aside). Only ever a built-in preset: the host itself is never used.
pub fn by_mx(host: &str) -> Option<usize> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    PRESETS.iter().position(|p| {
        p.mx_suffixes.iter().any(|s| {
            host == *s
                || host
                    .strip_suffix(s)
                    .is_some_and(|rest| rest.ends_with('.') && rest.len() > 1)
        })
    })
}

/// The index of the preset with this `id`.
pub fn index_of(id: &str) -> Option<usize> {
    PRESETS.iter().position(|p| p.id == id)
}

/// The preset with this `id` (the examples' and the tests' way to name one).
pub fn by_id(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// The default of "save a copy to Sent" for a new account on preset
/// `index` (`None`: another server, where the general default, on, holds).
pub fn save_sent_default(index: Option<usize>) -> bool {
    index
        .and_then(|i| PRESETS.get(i))
        .is_none_or(|p| p.save_sent)
}

/// The presets whose name or id contains `query`, or a domain starts with
/// it (case and accents aside, spaces trimmed), in list order; all for an
/// empty query.
pub fn search(query: &str) -> Vec<usize> {
    let q = fold(query.trim());
    PRESETS
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            q.is_empty()
                || fold(p.name).contains(&q)
                || p.id.contains(&q)
                || p.domains.iter().any(|d| d.starts_with(&q))
        })
        .map(|(i, _)| i)
        .collect()
}

/// Lower case, with the common Latin accents removed, for the search.
fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            'š' => 's',
            'ž' => 'z',
            'č' => 'c',
            'ř' => 'r',
            other => other,
        })
        .collect()
}

impl Preset {
    pub fn imap_server(&self) -> Server {
        Server {
            host: self.imap.0.into(),
            port: self.imap.1,
            security: self.imap.2,
        }
    }
    pub fn smtp_server(&self) -> Server {
        Server {
            host: self.smtp.0.into(),
            port: self.smtp.1,
            security: self.smtp.2,
        }
    }
    /// Whether picking it fills the servers in for good (a preset form, its
    /// servers not shown): not Bridge (its form stays editable) and not a
    /// provider Komorebi cannot sign in to.
    pub fn is_fixed(&self) -> bool {
        !matches!(self.sign_in, SignIn::Bridge | SignIn::NotSupported)
    }
    /// Whether it can be picked at all.
    pub fn is_supported(&self) -> bool {
        self.sign_in != SignIn::NotSupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(id: &str) -> usize {
        index_of(id).unwrap()
    }

    #[test]
    fn presets_are_well_formed() {
        let mut ids = Vec::new();
        let mut domains = Vec::new();
        for p in PRESETS {
            assert!(!ids.contains(&p.id), "{} twice", p.id);
            ids.push(p.id);
            assert!(!p.name.is_empty());
            for d in p.domains {
                assert_eq!(*d, d.to_ascii_lowercase(), "{d}");
                assert!(d.contains('.') && !d.contains('*'), "{d}");
                assert!(!domains.contains(d), "{d} in two presets");
                domains.push(*d);
            }
            for s in p.mx_suffixes {
                assert_eq!(*s, s.to_ascii_lowercase(), "{s}");
                assert!(s.contains('.') && !s.starts_with('.'), "{s}");
            }
            for (host, port, security) in [p.imap, p.smtp] {
                assert!(!host.is_empty() && port != 0, "{}", p.id);
                // Implicit TLS or STARTTLS, never plaintext; STARTTLS only
                // on the documented submission port or the Bridge's.
                if security == Security::StartTls {
                    assert!(port == 587 || p.sign_in == SignIn::Bridge, "{}", p.id);
                }
            }
            // Implicit TLS on the standard ports for every remote server.
            if p.sign_in != SignIn::Bridge {
                assert_eq!(p.imap, (p.imap.0, 993, Security::Tls), "{}", p.id);
            }
        }
        // Alphabetical, as the list shows them.
        let names: Vec<String> = PRESETS.iter().map(|p| p.name.to_lowercase()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn known_servers() {
        let p = by_id("purelymail").unwrap();
        assert_eq!(p.imap_server().host, "imap.purelymail.com");
        assert_eq!(p.smtp_server().host, "smtp.purelymail.com");
        let p = by_id("postale").unwrap();
        assert_eq!(
            p.imap_server(),
            Server {
                host: "mail.postale.io".into(),
                port: 993,
                security: Security::Tls
            }
        );
        assert_eq!(p.smtp_server().port, 465);
        let icloud = by_id("icloud").unwrap().smtp_server();
        assert_eq!((icloud.port, icloud.security), (587, Security::StartTls));
        let bridge = by_id("proton").unwrap();
        assert_eq!(bridge.imap_server().host, "127.0.0.1");
        assert!(!bridge.is_fixed() && bridge.is_supported());
        // Outlook.com and Microsoft 365 need Microsoft's own sign-in.
        for id in ["outlook", "microsoft365"] {
            assert!(!by_id(id).unwrap().is_supported(), "{id}");
        }
    }

    #[test]
    fn sent_copy_defaults_follow_the_preset() {
        // Postale.io does not save sent mail itself (live tests): on.
        assert!(by_id("postale").unwrap().save_sent);
        // Purelymail is undocumented: the general default.
        assert!(by_id("purelymail").unwrap().save_sent);
        // Gmail saves it itself.
        assert!(!save_sent_default(Some(at("gmail"))));
        assert!(save_sent_default(None));
        assert!(save_sent_default(Some(at("postale"))));
        assert!(save_sent_default(Some(99)));
    }

    #[test]
    fn the_provider_is_guessed_from_its_own_domain_only() {
        assert_eq!(guess("ana@purelymail.com"), Some(at("purelymail")));
        assert_eq!(guess(" Ana@PurelyMail.COM "), Some(at("purelymail")));
        assert_eq!(guess("ana@postale.io"), Some(at("postale")));
        assert_eq!(guess("ana@postale.io."), Some(at("postale")));
        assert_eq!(guess("ana@googlemail.com"), Some(at("gmail")));
        assert_eq!(guess("ana@me.com"), Some(at("icloud")));
        assert_eq!(guess("ana@hotmail.co.uk"), Some(at("outlook")));
        // A custom domain can be hosted by anyone: no guess.
        assert_eq!(guess("ana@example.com"), None);
        assert_eq!(guess("ana@mail.purelymail.com"), None);
        assert_eq!(guess("ana@purelymail.com.evil.example"), None);
        assert_eq!(guess("purelymail.com"), None);
        assert_eq!(guess(""), None);
        assert_eq!(guess("ana@"), None);
        assert!(on_own_domain(at("postale"), "a@postale.io"));
        assert!(!on_own_domain(at("purelymail"), "a@postale.io"));
    }

    #[test]
    fn mx_hosts_map_to_presets_by_whole_labels() {
        assert_eq!(by_mx("mail.postale.io."), Some(at("postale")));
        assert_eq!(by_mx("postale.io"), Some(at("postale")));
        assert_eq!(by_mx("ASPMX.L.GOOGLE.COM."), Some(at("google-workspace")));
        assert_eq!(by_mx("smtp.google.com"), Some(at("google-workspace")));
        assert_eq!(by_mx("aspmx2.googlemail.com"), Some(at("google-workspace")));
        assert_eq!(
            by_mx("example-com.mail.protection.outlook.com"),
            Some(at("microsoft365"))
        );
        assert_eq!(by_mx("in1-smtp.messagingengine.com"), Some(at("fastmail")));
        assert_eq!(by_mx("mail.protonmail.ch"), Some(at("proton")));
        assert_eq!(by_mx("mx01.mail.icloud.com"), Some(at("icloud")));
        assert_eq!(by_mx("mx2.zoho.com"), Some(at("zoho-business")));
        assert_eq!(by_mx("mxext1.mailbox.org"), Some(at("mailboxorg")));
        assert_eq!(by_mx("mailserver.purelymail.com"), Some(at("purelymail")));
        assert_eq!(by_mx("mta-gw.infomaniak.ch"), Some(at("infomaniak")));
        assert_eq!(by_mx("mx.yandex.net"), Some(at("yandex")));
        // Not a label boundary, someone else's name, or nothing.
        assert_eq!(by_mx("evilpostale.io"), None);
        assert_eq!(by_mx("postale.io.evil.example"), None);
        assert_eq!(by_mx("mx.example.com"), None);
        assert_eq!(by_mx(".postale.io"), None);
        assert_eq!(by_mx(""), None);
        assert_eq!(by_mx("."), None);
    }

    #[test]
    fn search_matches_names_and_domains_in_list_order() {
        let names = |q: &str| -> Vec<&str> { search(q).iter().map(|&i| PRESETS[i].name).collect() };
        assert_eq!(search("").len(), PRESETS.len());
        assert_eq!(search("   ").len(), PRESETS.len());
        assert_eq!(names("gmail"), ["Gmail"]);
        assert_eq!(names("GOOGLE"), ["Gmail", "Google Workspace"]);
        assert_eq!(names("hotmail"), ["Outlook.com"]);
        assert_eq!(
            names("zoho"),
            ["Zoho Mail", "Zoho Mail EU", "Zoho Mail for business"]
        );
        assert_eq!(names("me.com"), ["iCloud Mail"]);
        assert_eq!(names("prõton"), ["Proton Mail"]);
        assert!(search("nothing like this").is_empty());
    }
}
