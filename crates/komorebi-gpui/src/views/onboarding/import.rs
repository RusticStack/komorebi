//! Import from Outlook (the setup flow's list, `Step::Import`): the plain
//! rules behind it. Which found accounts start ticked (not one already
//! here), how a ticked account fills the Sign in step, and the signatures
//! hand-off ([`read_signatures`], [`save_signatures`]). The finding itself is
//! `komorebi::import::outlook`; nothing here reads Outlook.

use komorebi::find::Fields;
use komorebi::import::outlook::{self, FoundAccount, FoundSignature, Kind};
use komorebi::providers::{self as presets, PRESETS, Security};

/// A found account on the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountRow {
    pub account: FoundAccount,
    /// Already an account in Komorebi: marked, unticked.
    pub exists: bool,
    pub ticked: bool,
}

/// A found signature on the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureRow {
    pub signature: FoundSignature,
    pub ticked: bool,
}

/// The list's rows: every account, ticked unless its address is already
/// here (`existing`); every signature, ticked.
pub fn rows(found: &outlook::Found, existing: &[String]) -> (Vec<AccountRow>, Vec<SignatureRow>) {
    let accounts = found
        .accounts
        .iter()
        .map(|a| {
            let exists = outlook::already_added(&a.email, existing.iter().map(String::as_str));
            AccountRow {
                account: a.clone(),
                exists,
                ticked: !exists,
            }
        })
        .collect();
    let signatures = found
        .signatures
        .iter()
        .map(|s| SignatureRow {
            signature: s.clone(),
            ticked: true,
        })
        .collect();
    (accounts, signatures)
}

/// How a ticked account fills the Sign in step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prefill {
    /// A built-in provider (`PRESETS` index): its servers; one Komorebi
    /// cannot sign in to yet shows why, with no password field.
    Preset(usize),
    /// Outlook's own IMAP and SMTP settings, on manual setup.
    Manual(Fields),
    /// No IMAP settings to take (a POP account on no known provider): the
    /// lookup, or the provider list offline, as for a typed address.
    Lookup,
}

/// The Microsoft provider an Exchange account maps to: Outlook.com for its
/// domains, otherwise Microsoft 365.
fn microsoft(email: &str) -> usize {
    presets::guess(email)
        .or_else(|| presets::index_of("microsoft365"))
        .or_else(|| presets::index_of("outlook"))
        .unwrap_or(0)
}

/// What an account fills in: a provider its domain belongs to; for IMAP,
/// otherwise Outlook's servers and sign-in names; for Exchange, the
/// Microsoft provider; for POP, a lookup.
pub fn prefill(a: &FoundAccount) -> Prefill {
    match a.kind {
        Kind::Exchange => Prefill::Preset(microsoft(&a.email)),
        _ if presets::guess(&a.email).is_some() => {
            Prefill::Preset(presets::guess(&a.email).unwrap_or(0))
        }
        Kind::Pop => Prefill::Lookup,
        Kind::Imap => {
            let Some(imap) = &a.incoming else {
                return Prefill::Lookup;
            };
            let smtp = a.smtp.clone();
            let same = |u: &str| u.is_empty() || u.eq_ignore_ascii_case(&a.email);
            let imap_user = if same(&a.incoming_user) {
                String::new()
            } else {
                a.incoming_user.clone()
            };
            let smtp_own = !a.smtp_user.is_empty() && a.smtp_user != a.incoming_user;
            Prefill::Manual(Fields {
                imap_host: imap.host.clone(),
                imap_port: imap.port.to_string(),
                imap_starttls: imap.security == Security::StartTls,
                imap_user: imap_user.clone(),
                smtp_host: smtp.as_ref().map(|s| s.host.clone()).unwrap_or_default(),
                smtp_port: smtp
                    .as_ref()
                    .map(|s| s.port.to_string())
                    .unwrap_or_default(),
                smtp_starttls: smtp
                    .as_ref()
                    .is_some_and(|s| s.security == Security::StartTls),
                smtp_user: if smtp_own {
                    a.smtp_user.clone()
                } else {
                    String::new()
                },
                smtp_own_login: smtp_own,
            })
        }
    }
}

/// The sign-in name for a provider's Username field: Outlook's, when it is
/// not the address.
pub fn preset_user(a: &FoundAccount) -> Option<String> {
    let u = a.incoming_user.trim();
    (!u.is_empty() && !u.eq_ignore_ascii_case(&a.email)).then(|| u.to_string())
}

/// Whether Komorebi cannot sign in to this account yet (its provider allows
/// only its own sign-in).
pub fn unsupported(a: &FoundAccount) -> bool {
    matches!(prefill(a), Prefill::Preset(i) if PRESETS.get(i).is_some_and(|p| !p.is_supported()))
}

/// A signature name not taken yet: `name`, else `name (2)`, `name (3)`…
fn free_name(db: &komorebi::db::Db, name: &str) -> String {
    let taken: Vec<String> = db
        .signatures()
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.name.to_lowercase())
        .collect();
    let base = name.trim();
    let base = if base.is_empty() { "Outlook" } else { base };
    if !taken.contains(&base.to_lowercase()) {
        return base.to_string();
    }
    (2..1000)
        .map(|n| format!("{base} ({n})"))
        .find(|n| !taken.contains(&n.to_lowercase()))
        .unwrap_or_else(|| format!("{base} ({})", chrono::Utc::now().timestamp()))
}

// ==== SIGNATURE IMPORT SEAM ==================================================
//
// The one place found Outlook signatures become Komorebi signatures: read
// with their layout (tables, merged cells, colours) and their pictures on
// this PC (`outlook::to_signature`, off the UI thread), then saved with them.
// A picture on the web is never fetched.

/// An Outlook signature read, ready to save: its Outlook name, its content
/// and its pictures.
pub struct ReadSignature {
    pub name: String,
    pub doc: komorebi::mail::rich::Doc,
    pub images: komorebi::mail::rich::Images,
}

/// Read each ticked Outlook signature and the picture files it refers to.
/// Reads files: run it off the UI thread. One with nothing readable is left
/// out.
pub fn read_signatures(found: Vec<FoundSignature>) -> Vec<ReadSignature> {
    found
        .into_iter()
        .filter_map(|sig| {
            let (doc, images) =
                outlook::to_signature(&sig, &komorebi::mail::rich::images::read_small_file)?;
            Some(ReadSignature {
                name: sig.name,
                doc,
                images,
            })
        })
        .collect()
}

/// Save the read signatures as Komorebi signatures (each under its Outlook
/// name, made unique), with their pictures. Returns `(Outlook name, new
/// signature id)` for each one saved, for the accounts' defaults.
pub fn save_signatures(db: &komorebi::db::Db, read: Vec<ReadSignature>) -> Vec<(String, i64)> {
    let mut saved = Vec::new();
    for sig in read {
        let name = free_name(db, &sig.name);
        if let Ok(id) = db.add_signature(&name, &sig.doc, &sig.images) {
            saved.push((sig.name, id));
        }
    }
    saved
}

// ============================================================================

/// The new account's defaults from Outlook's (new mail, replies and
/// forwards), when those signatures were imported. Writes nothing when
/// neither was.
pub fn apply_defaults(
    db: &komorebi::db::Db,
    account_id: i64,
    a: &FoundAccount,
    imported: &[(String, i64)],
) {
    let id_of = |name: &Option<String>| {
        name.as_ref().and_then(|n| {
            imported
                .iter()
                .find(|(s, _)| s.eq_ignore_ascii_case(n))
                .map(|(_, id)| *id)
        })
    };
    let (new, reply) = (id_of(&a.new_signature), id_of(&a.reply_signature));
    if new.is_some() || reply.is_some() {
        let _ = db.set_account_signatures(account_id, new, reply);
    }
}

/// Made-up findings for the `ob-import` snapshot: never this PC's.
pub fn sample() -> outlook::Found {
    use komorebi::providers::Server;
    let server = |host: &str, port, security| {
        Some(Server {
            host: host.into(),
            port,
            security,
        })
    };
    let account = |kind, email: &str, name: &str| FoundAccount {
        kind,
        profile: "Outlook".into(),
        account_name: email.into(),
        display_name: name.into(),
        email: email.into(),
        incoming: None,
        incoming_user: String::new(),
        smtp: None,
        smtp_user: String::new(),
        new_signature: None,
        reply_signature: None,
    };
    outlook::Found {
        accounts: vec![
            FoundAccount {
                incoming: server("imap.example.org", 993, Security::Tls),
                smtp: server("smtp.example.org", 587, Security::StartTls),
                new_signature: Some("Work".into()),
                ..account(Kind::Imap, "ana.silva@example.org", "Ana Silva")
            },
            FoundAccount {
                incoming: server("pop.example.net", 995, Security::Tls),
                smtp: server("smtp.example.net", 465, Security::Tls),
                ..account(Kind::Pop, "ana@example.net", "Ana Silva")
            },
            account(Kind::Exchange, "ana.silva@contoso.example", "Ana Silva"),
        ],
        signatures: vec![
            FoundSignature {
                name: "Work".into(),
                html: "<p>Ana Silva</p><p>Example Ltd</p>\
                       <p><img src=\"https://example.org/award.png\" alt=\"Award\"></p>"
                    .into(),
                charset: "windows-1252".into(),
                images: vec!["Work_files/image001.png".into()],
            },
            FoundSignature {
                name: "Short".into(),
                html: "<p>Ana</p>".into(),
                charset: "windows-1252".into(),
                images: Vec::new(),
            },
        ],
        new_outlook_only: false,
    }
}

#[cfg(test)]
mod tests {
    use komorebi::providers::Server;

    use super::*;

    fn account(kind: Kind, email: &str) -> FoundAccount {
        FoundAccount {
            kind,
            profile: "Outlook".into(),
            account_name: email.into(),
            display_name: "Ana Silva".into(),
            email: email.into(),
            incoming: None,
            incoming_user: String::new(),
            smtp: None,
            smtp_user: String::new(),
            new_signature: None,
            reply_signature: None,
        }
    }

    fn imap(email: &str) -> FoundAccount {
        FoundAccount {
            incoming: Some(Server {
                host: "imap.example.org".into(),
                port: 993,
                security: Security::Tls,
            }),
            incoming_user: "ana".into(),
            smtp: Some(Server {
                host: "smtp.example.org".into(),
                port: 587,
                security: Security::StartTls,
            }),
            ..account(Kind::Imap, email)
        }
    }

    #[test]
    fn known_addresses_are_marked_and_unticked() {
        let found = outlook::Found {
            accounts: vec![imap("ana@example.org"), imap("rui@example.org")],
            signatures: vec![FoundSignature {
                name: "Work".into(),
                html: "<p>Ana</p>".into(),
                charset: "utf-8".into(),
                images: Vec::new(),
            }],
            new_outlook_only: false,
        };
        let (a, s) = rows(&found, &["ANA@example.org".to_string()]);
        assert_eq!(
            a.iter().map(|r| (r.exists, r.ticked)).collect::<Vec<_>>(),
            [(true, false), (false, true)]
        );
        assert!(s[0].ticked);
    }

    #[test]
    fn imap_accounts_fill_manual_setup() {
        let Prefill::Manual(f) = prefill(&imap("ana@example.org")) else {
            panic!("manual");
        };
        assert_eq!(f.imap_host, "imap.example.org");
        assert_eq!(f.imap_port, "993");
        assert!(!f.imap_starttls);
        assert_eq!(f.imap_user, "ana");
        assert_eq!(f.smtp_host, "smtp.example.org");
        assert_eq!(f.smtp_port, "587");
        assert!(f.smtp_starttls);
        assert!(!f.smtp_own_login);
        // A separate sending sign-in.
        let own = FoundAccount {
            smtp_user: "ana-send".into(),
            ..imap("ana@example.org")
        };
        let Prefill::Manual(f) = prefill(&own) else {
            panic!("manual");
        };
        assert!(f.smtp_own_login);
        assert_eq!(f.smtp_user, "ana-send");
    }

    #[test]
    fn providers_exchange_and_pop_map_as_documented() {
        let gmail = imap("ana@gmail.com");
        assert_eq!(
            prefill(&gmail),
            Prefill::Preset(presets::index_of("gmail").unwrap())
        );
        assert_eq!(preset_user(&gmail).as_deref(), Some("ana"));
        let hotmail = account(Kind::Exchange, "ana@hotmail.com");
        assert_eq!(
            prefill(&hotmail),
            Prefill::Preset(presets::index_of("outlook").unwrap())
        );
        assert!(unsupported(&hotmail));
        let work = account(Kind::Exchange, "ana@example.org");
        assert_eq!(
            prefill(&work),
            Prefill::Preset(presets::index_of("microsoft365").unwrap())
        );
        assert!(unsupported(&work));
        assert_eq!(
            prefill(&account(Kind::Pop, "old@example.org")),
            Prefill::Lookup
        );
        assert!(!unsupported(&imap("ana@example.org")));
    }
}
