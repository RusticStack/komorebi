//! Settings' and the account form's words (docs/GPUI-I18N.md): each the
//! Slint app's catalog entry (`ui/tr.slint`, `ui/app.slint`, `ui/forms.slint`),
//! so the template only gains file names. Views keep typed values (a
//! [`FormError`], a [`Say`]) and call these in `render`; a language switch
//! redraws them.

use komorebi::providers;
use komorebi_i18n::{tr, trn};

use super::hosts::FormError;

/// Why the account form can't be used, in a sentence (`text::form_error`).
pub fn form_error(e: FormError) -> String {
    match e {
        FormError::Email => err_email(),
        FormError::Provider => err_provider(),
        FormError::ServerNames => err_server_names(),
        FormError::ServerNameOnly => err_server_name_only(),
        FormError::ZoneId => err_zone_id(),
        FormError::AsciiName => err_ascii_name(),
        FormError::Port(p) => err_port(p.name()),
        FormError::Username(p) => err_username(p.name()),
    }
}

/// A rejected password in the account form, by provider (`preset`: the
/// index of the one chosen; `None`: Other server, or an edited account): a
/// preset gets the app-password advice only, and, for an address on
/// another domain than its own, a pointer to the other preset; any other
/// server also the username advice (`text::rejected`).
pub fn rejected(preset: Option<usize>, email: &str, host: &str, login: &str) -> String {
    let Some(index) = preset.filter(|&i| i < providers::PRESETS.len()) else {
        return say_rejected(host, login);
    };
    let advice = say_rejected_preset(host, login);
    if providers::on_own_domain(index, email) {
        return advice;
    }
    match providers::PRESETS
        .iter()
        .enumerate()
        .find(|(i, _)| *i != index)
    {
        Some((_, other)) => format!("{advice} {}", say_other_preset(other.name)),
        None => advice,
    }
}

// The Slint app's `Tr` functions, word for word (generated from
// `ui/tr.slint`; the msgid and context are the catalog's).

pub fn err_email() -> String {
    tr!("Enter your full email address.")
}

pub fn err_provider() -> String {
    tr!("Choose your provider.")
}

pub fn err_server_names() -> String {
    tr!("Enter the IMAP and SMTP server names.")
}

pub fn err_server_name_only() -> String {
    tr!("Enter the server name only, such as mail.example.com.")
}

pub fn err_zone_id() -> String {
    tr!("Zone identifiers such as %eth0 aren't supported. Remove the part after the % sign.")
}

pub fn err_ascii_name() -> String {
    tr!("Use the ASCII (xn--) form of the server name, such as xn--mail-example.com.")
}

pub fn err_port(proto: &str) -> String {
    tr!("Enter a valid {0} port.", proto)
}

pub fn err_username(proto: &str) -> String {
    tr!("Enter a valid {0} username.", proto)
}

pub fn err_account_exists() -> String {
    tr!("This account is already added.")
}

pub fn err_account_removed() -> String {
    tr!("This account was removed.")
}

pub fn err_password_to_test() -> String {
    tr!("Enter the password to test.")
}

pub fn err_offline_check() -> String {
    tr!("Offline: a password or server change can't be checked.")
}

pub fn err_offline_cert() -> String {
    tr!("Offline: a certificate can't be reviewed.")
}

pub fn err_offline_find() -> String {
    tr!("Offline: settings can't be looked up.")
}

pub fn err_saved_password() -> String {
    tr!("Komorebi couldn't read the saved password. Type the password to test.")
}

pub fn cert_pinned(fingerprint: &str) -> String {
    tr!("Certificate: pinned (SHA-256 {0})", fingerprint)
}

pub fn cert_pinned_key(fingerprint: &str) -> String {
    tr!("Certificate: server key pinned (SHA-256 {0})", fingerprint)
}

pub fn cert_pinned_ca(issuer: &str) -> String {
    tr!("Certificate: issued by the pinned authority {0}", issuer)
}

pub fn cert_damaged(host: &str) -> String {
    tr!(
        "The saved certificate for {0} is damaged. Forget it and review the certificate again.",
        host
    )
}

pub fn cert_windows() -> String {
    tr!("Certificate: checked by Windows")
}

pub fn cert_unreadable(address: &str) -> String {
    tr!(
        "Couldn't read the certificate of {0}. Check the server name, the port and the security setting, then try again.",
        address
    )
}

pub fn cert_trusted(host: &str) -> String {
    tr!(
        "Certificate trusted for {0}. Press Test connection, or Add account or Save, to continue.",
        host
    )
}

pub fn cert_names(names: &str) -> String {
    tr!("Names: {0}", names)
}

pub fn cert_subject(subject: &str) -> String {
    tr!("Subject: {0}", subject)
}

pub fn cert_self_signed() -> String {
    tr!("Self-signed")
}

pub fn cert_issued_by(issuer: &str) -> String {
    tr!("Issued by {0}", issuer)
}

pub fn cert_valid(begin: &str, end: &str) -> String {
    tr!("Valid from {0} to {1}", begin, end)
}

pub fn cert_fp_label() -> String {
    tr!("SHA-256 fingerprint")
}

pub fn cert_fp_new() -> String {
    tr!("New certificate (SHA-256)")
}

pub fn cert_fp_old() -> String {
    tr!("Saved before (SHA-256)")
}

pub fn cert_fp_caution() -> String {
    tr!("This isn't the certificate you saved. If you didn't expect a new one, cancel.")
}

pub fn q_trust_title() -> String {
    tr!("Trust this certificate?")
}

pub fn q_trust_text(address: &str, host: &str) -> String {
    tr!(
        "{0} presented a certificate that Windows doesn't trust. Trust this certificate for {1} only?",
        address,
        host
    )
}

pub fn q_trust_ok() -> String {
    tr!("Trust this certificate")
}

pub fn trust_cert() -> String {
    tr!("This exact certificate")
}

pub fn trust_cert_note() -> String {
    tr!("Any other certificate is refused, including a renewal.")
}

pub fn trust_key() -> String {
    tr!("This server key")
}

pub fn trust_key_note(fingerprint: &str) -> String {
    tr!(
        "A renewed certificate with the same key is accepted. Key SHA-256: {0}",
        fingerprint
    )
}

pub fn trust_ca(issuer: &str) -> String {
    tr!("The issuing CA: {0}", issuer)
}

pub fn trust_ca_note(fingerprint: &str) -> String {
    tr!(
        "Certificates it issues for this server name are accepted. CA SHA-256: {0}",
        fingerprint
    )
}

pub fn q_trust_note() -> String {
    tr!(
        "Ask whoever runs the server for its fingerprint, or run openssl x509 -noout -fingerprint -sha256 on its certificate, and compare every byte. Only trust it on a network you control. A saved certificate has to be reviewed again when the server renews it."
    )
}

pub fn check_passed() -> String {
    tr!("Every step passed. Nothing was saved; use Add account or Save to keep these settings.")
}

pub fn account_saved(email: &str) -> String {
    tr!("Saved {0}.", email)
}

pub fn not_saved(detail: &str) -> String {
    tr!("Not saved. {0}", detail)
}

pub fn not_removed(detail: &str) -> String {
    tr!("Not removed. {0}", detail)
}

pub fn not_reset(detail: &str) -> String {
    tr!("Not reset. {0}", detail)
}

pub fn redownloading(email: &str) -> String {
    tr!("Downloading the mail of {0} again\u{a0}…", email)
}

pub fn redownload_later(email: &str) -> String {
    tr!("The mail of {0} downloads again at its next sync.", email)
}

pub fn account_sign_in_rejected() -> String {
    tr!("Sign-in rejected")
}

pub fn account_secure_failed() -> String {
    tr!("Secure connection failed")
}

pub fn account_syncing() -> String {
    tr!("Syncing\u{a0}…")
}

pub fn account_checked(time: &str) -> String {
    tr!("Up to date, checked {0}", time)
}

pub fn account_up_to_date() -> String {
    tr!("Up to date")
}

pub fn account_unreachable(host: &str) -> String {
    tr!("Can't reach {0}", host)
}

pub fn account_sync_failed(detail: &str) -> String {
    tr!("Last sync failed: {0}", detail)
}

pub fn role_automatic(name: &str) -> String {
    tr!("Automatic ({0})", name)
}

pub fn role_automatic_none() -> String {
    tr!("Automatic (none found)")
}

pub fn q_remove_title(email: &str) -> String {
    tr!("Remove {0}?", email)
}

pub fn q_remove_text() -> String {
    tr!(
        "Removes this account and its downloaded mail from this computer. Mail on the server is not affected."
    )
}

pub fn q_remove_unsent(n: usize) -> String {
    trn!(
        "Its {n} unsent draft or Outbox message will be lost.",
        "Its {n} unsent drafts and Outbox messages will be lost.",
        n
    )
}

pub fn q_remove_ok() -> String {
    tr!("Remove")
}

pub fn q_redownload_title(email: &str) -> String {
    tr!("Download the mail of {0} again?", email)
}

pub fn q_redownload_text() -> String {
    tr!(
        "Komorebi forgets the mail it downloaded for this account and downloads it again. Mail on the server is not affected; drafts, the Outbox and settings stay."
    )
}

pub fn q_redownload_ok() -> String {
    tr!("Download again")
}

pub fn q_wipe_title() -> String {
    tr!("Remove all of Komorebi's data?")
}

pub fn q_wipe_text() -> String {
    tr!(
        "Deletes the saved passwords of all accounts, then the downloaded mail, drafts, Outbox, settings and logs on this computer, and closes Komorebi. Mail on the server is not affected."
    )
}

pub fn q_wipe_unsent(n: usize) -> String {
    trn!(
        "{n} unsent draft or Outbox message will be lost.",
        "{n} unsent drafts and Outbox messages will be lost.",
        n
    )
}

pub fn q_wipe_ok() -> String {
    tr!("Remove all data")
}

pub fn wipe_files_left(list: &str) -> String {
    tr!(
        "The passwords were deleted, but these files are still in use and were not removed. Delete them by hand:\n\n{0}",
        list
    )
}

pub fn cache_damaged(name: &str) -> String {
    tr!(
        "The mail cache was damaged, so Komorebi started a new one; the old file is kept as {0}. Add your accounts again in Settings.",
        name
    )
}

pub fn sig_name_empty() -> String {
    tr!("Give the signature a name.")
}

pub fn sig_name_taken() -> String {
    tr!("A signature with this name already exists.")
}

pub fn sig_logo_refused() -> String {
    tr!("The logo must be a PNG or JPEG file of 200 KB or less.")
}

pub fn sig_none() -> String {
    tr!("signature" => "None")
}

pub fn q_sig_delete_title(name: &str) -> String {
    tr!("Delete signature \u{201c}{0}\u{201d}?", name)
}

pub fn q_sig_delete_text() -> String {
    tr!("Messages already written keep it.")
}

pub fn q_sig_delete_users(n: usize) -> String {
    trn!(
        "It is the default for {n} account; that account gets no signature.",
        "It is the default for {n} accounts; those accounts get no signature.",
        n
    )
}

pub fn find_none() -> String {
    tr!("No settings found, enter them by hand.")
}

pub fn find_builtin() -> String {
    tr!("Komorebi has settings built in for this provider.")
}

pub fn find_srv() -> String {
    tr!("Found in your domain's DNS records.")
}

pub fn find_autoconfig() -> String {
    tr!("Found by your provider's autoconfig.")
}

pub fn find_both() -> String {
    tr!("Found in your domain's DNS records and by your provider's autoconfig.")
}

pub fn find_no_incoming() -> String {
    tr!("No incoming server found, enter it by hand.")
}

pub fn find_no_sending() -> String {
    tr!("No sending server found, enter it by hand.")
}

pub fn find_refused(host: &str) -> String {
    tr!(
        "The provider's settings point to {0}, a different domain. Not filled in.",
        host
    )
}

pub fn find_kept() -> String {
    tr!("Kept the server settings you typed.")
}

pub fn account_line(email: &str, text: &str) -> String {
    tr!("{0}: {1}", email, text)
}

pub fn error_detail(detail: &str) -> String {
    tr!("Something went wrong. {0}", detail)
}

pub fn say_rejected(host: &str, login: &str) -> String {
    tr!(
        "{0} rejected the password for {1}. If sign-in uses two steps, create an app password. If the login isn't your email address, set the username.",
        host,
        login
    )
}

pub fn say_rejected_preset(host: &str, login: &str) -> String {
    tr!(
        "{0} rejected the password for {1}. If sign-in uses two steps, create an app password.",
        host,
        login
    )
}

pub fn say_other_preset(provider: &str) -> String {
    tr!("Is your mail at {0}? Change Provider.", provider)
}

// ---- Words of the page chrome (the Slint app's `ui/app.slint`).

pub fn settings_title() -> String {
    tr!("Settings")
}

pub fn keyboard_shortcuts_title() -> String {
    tr!("Keyboard shortcuts")
}

pub fn close() -> String {
    tr!("Close")
}
