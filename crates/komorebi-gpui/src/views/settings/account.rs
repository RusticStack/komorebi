//! What adding, saving and removing an account does to the cache and to
//! Windows Credential Manager (the Slint app's `add_account`,
//! `finish_add_account`, `update_account`, `finish_update_account` and
//! `remove_account_now`), as plain functions with the password store passed
//! in, so the rules are tested without a window or the real credential
//! store. The sync workers are the shell's: these functions only say what
//! changed ([`Change`]).

use anyhow::Result;
use komorebi::db::{Account, Db, NewAccount};

use super::form::AccountForm;
use super::hosts::FormError;

/// Saves an account's password (`secrets::set_password`; tests: a fake).
pub type StorePassword<'a> = &'a dyn Fn(i64, &str) -> Result<()>;

/// Forgets an account's password (`secrets::delete_password`; tests: a fake).
pub type ForgetPassword<'a> = &'a dyn Fn(i64) -> Result<()>;

/// The certificates an account's servers are pinned to (IMAP, SMTP).
pub type Pins = (Option<Vec<u8>>, Option<Vec<u8>>);

/// Add the account `new` once its check passed: the row, its password in
/// the credential store, then its signature and pins. If any part fails,
/// nothing is left behind (no row, no credential).
pub fn add(
    db: &Db,
    new: &NewAccount,
    signature: &str,
    password: &str,
    pins: Pins,
    store: StorePassword,
    forget: ForgetPassword,
) -> Result<i64> {
    let id = db.add_account(new)?;
    let with_signature = Account {
        id,
        email: new.email.clone(),
        name: new.name.clone(),
        imap: new.imap.clone(),
        smtp: new.smtp.clone(),
        save_sent: new.save_sent,
        signature: signature.to_string(),
        enabled: true,
        imap_user: new.imap_user.clone(),
        smtp_user: new.smtp_user.clone(),
        imap_pin: pins.0,
        smtp_pin: pins.1,
        smtp_no_auth: new.smtp_no_auth,
    };
    store(id, password)
        .and_then(|()| db.update_account(&with_signature).map(drop))
        .inspect_err(|_| {
            forget(id).ok();
            db.remove_account(id).ok();
        })?;
    Ok(id)
}

/// The account an edited form makes (the Slint app's `update_account`
/// before its check): the name, servers, sign-in names, pins, Sent copy,
/// signature and sign-in choice from the form, everything else as it was.
pub fn edited(old: &Account, form: &AccountForm, pins: &Pins) -> Result<Account, FormError> {
    let (imap, smtp) = form.servers()?;
    let (imap_user, smtp_user) = form.logins(&old.email)?;
    Ok(Account {
        name: form.name.trim().to_string(),
        imap,
        smtp,
        imap_user,
        smtp_user,
        imap_pin: pins.0.clone(),
        smtp_pin: pins.1.clone(),
        save_sent: form.save_sent,
        signature: form.signature.clone(),
        smtp_no_auth: form.no_auth(),
        ..old.clone()
    })
}

/// Whether saving `edited` over `old` needs the servers checked first: a
/// new password, or anything about how the servers are reached.
pub fn needs_check(old: &Account, edited: &Account, new_password: bool) -> bool {
    new_password || !same_servers(old, edited)
}

fn same_servers(old: &Account, edited: &Account) -> bool {
    edited.imap == old.imap
        && edited.smtp == old.smtp
        && edited.imap_user == old.imap_user
        && edited.smtp_user == old.smtp_user
        && edited.imap_pin == old.imap_pin
        && edited.smtp_pin == old.smtp_pin
        && edited.smtp_no_auth == old.smtp_no_auth
}

/// What saving an edited account changed, for the shell's workers: the
/// workers hold the account as it was, so a change of servers needs new
/// ones; a new password only lifts the sign-in latch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub servers: bool,
    pub password: bool,
}

/// Why an edited account was not saved.
#[derive(Debug)]
pub enum SaveError {
    /// Removed while the check ran: never save a password for it.
    Gone,
    Store(anyhow::Error),
}

/// Save `edited` (and its new `password`, if any) over `old` after its
/// check passed.
pub fn save(
    db: &Db,
    old: &Account,
    edited: &Account,
    password: Option<&str>,
    store: StorePassword,
) -> Result<Change, SaveError> {
    // Removed while the check ran.
    let present = db
        .accounts()
        .map_err(SaveError::Store)?
        .iter()
        .any(|a| a.id == edited.id);
    if !present {
        return Err(SaveError::Gone);
    }
    if let Some(p) = password {
        store(edited.id, p).map_err(SaveError::Store)?;
    }
    db.update_account(edited).map_err(SaveError::Store)?;
    Ok(Change {
        servers: edited.imap != old.imap
            || edited.smtp != old.smtp
            || edited.imap_user != old.imap_user
            || edited.smtp_user != old.smtp_user
            || edited.imap_pin != old.imap_pin
            || edited.smtp_pin != old.smtp_pin
            || edited.save_sent != old.save_sent,
        password: password.is_some(),
    })
}

/// Remove an account: its password first (a credential is never left
/// behind for an account that is gone), then the row with its mail,
/// drafts and Outbox.
pub fn remove(db: &Db, id: i64, forget: ForgetPassword) -> Result<()> {
    forget(id)?;
    db.remove_account(id)
}

/// How many unsent drafts and Outbox messages go with account `id`
/// (`None`: all accounts), for the question that asks first.
pub fn unsent(db: &Db, id: Option<i64>) -> usize {
    let count = |id| db.count_unsent(id).unwrap_or(0).max(0) as usize;
    match id {
        Some(id) => count(id),
        None => db
            .accounts()
            .unwrap_or_default()
            .iter()
            .map(|a| count(a.id))
            .sum(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use komorebi::providers::{Security, Server};

    use super::*;
    use crate::test_support::TempCache;

    fn server(host: &str) -> Server {
        Server {
            host: host.into(),
            port: 993,
            security: Security::Tls,
        }
    }

    fn new_account(email: &str) -> NewAccount {
        NewAccount {
            email: email.into(),
            name: String::new(),
            imap: server("imap.example.org"),
            smtp: server("smtp.example.org"),
            save_sent: true,
            imap_user: String::new(),
            smtp_user: String::new(),
            smtp_no_auth: false,
        }
    }

    #[test]
    fn an_account_is_added_with_its_password_and_a_failed_store_leaves_nothing() {
        let cache = TempCache::new("settings-add");
        let db = cache.open();
        let stored = RefCell::new(Vec::new());
        let store = |id: i64, p: &str| {
            stored.borrow_mut().push((id, p.to_string()));
            Ok(())
        };
        let forgotten = RefCell::new(Vec::new());
        let forget = |id: i64| {
            forgotten.borrow_mut().push(id);
            Ok(())
        };
        let id = add(
            &db,
            &new_account("a@example.org"),
            "Ana",
            "pw",
            (None, None),
            &store,
            &forget,
        )
        .unwrap();
        assert_eq!(*stored.borrow(), [(id, "pw".to_string())]);
        let a = db.accounts().unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].signature, "Ana");
        // A credential store that refuses: no row stays, the credential is
        // forgotten.
        let refuse = |_: i64, _: &str| anyhow::bail!("no credential store");
        let err = add(
            &db,
            &new_account("b@example.org"),
            "",
            "pw",
            (None, None),
            &refuse,
            &forget,
        );
        assert!(err.is_err());
        assert_eq!(db.accounts().unwrap().len(), 1);
        assert_eq!(forgotten.borrow().len(), 1);
    }

    #[test]
    fn saving_says_what_changed_and_never_saves_for_a_removed_account() {
        let cache = TempCache::new("settings-save");
        let db = cache.open();
        let ok = |_: i64, _: &str| Ok(());
        let id = add(
            &db,
            &new_account("a@example.org"),
            "",
            "pw",
            (None, None),
            &ok,
            &|_| Ok(()),
        )
        .unwrap();
        let old = db.accounts().unwrap().remove(0);
        let mut form = AccountForm::of(&old);
        form.name = "Ana".into();
        let edit = edited(&old, &form, &(None, None)).unwrap();
        assert!(!needs_check(&old, &edit, false), "a name needs no check");
        assert!(needs_check(&old, &edit, true), "a new password does");
        let change = save(&db, &old, &edit, None, &ok).unwrap();
        assert_eq!(
            change,
            Change {
                servers: false,
                password: false
            }
        );
        assert_eq!(db.accounts().unwrap()[0].name, "Ana");
        // Another server: a check and new workers.
        form.imap_host = "imap.other.org".into();
        let moved = edited(&db.accounts().unwrap()[0], &form, &(None, None)).unwrap();
        assert!(needs_check(&edit, &moved, false));
        assert!(save(&db, &edit, &moved, Some("new"), &ok).unwrap().servers);
        // Removed while a check ran.
        remove(&db, id, &|_| Ok(())).unwrap();
        assert!(matches!(
            save(&db, &moved, &moved, Some("x"), &ok),
            Err(SaveError::Gone)
        ));
    }

    #[test]
    fn removing_forgets_the_password_first_and_stops_if_it_cannot() {
        let cache = TempCache::new("settings-remove");
        let db = cache.open();
        let id = add(
            &db,
            &new_account("a@example.org"),
            "",
            "pw",
            (None, None),
            &|_, _| Ok(()),
            &|_| Ok(()),
        )
        .unwrap();
        let refuse = |_: i64| anyhow::bail!("Windows Credential Manager is not available");
        assert!(remove(&db, id, &refuse).is_err());
        assert_eq!(db.accounts().unwrap().len(), 1, "the account stays");
        assert!(remove(&db, id, &|_| Ok(())).is_ok());
        assert!(db.accounts().unwrap().is_empty());
        assert_eq!(unsent(&db, None), 0);
    }
}
