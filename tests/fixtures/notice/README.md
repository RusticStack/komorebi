# Sender-notice, encrypted-mail and unsubscribe fixtures

Ten synthetic messages for `tests/notice_corpus.rs` and snapshot QA
(`seed -- <new.sqlite3> 0 tests/fixtures/notice`; `KOMOREBI_SNAPSHOT_SELECT=n`
opens file n+1). **None of it is real mail**: addresses use `example.*` and
`komorebi.invalid` domains, `Authentication-Results` headers are written by
hand with the authserv-id `mx.komorebi.invalid` (the seed account's IMAP host
is `mail.komorebi.invalid`), and the "ciphertext" is placeholder base64 text,
not real S/MIME or OpenPGP data.

| File | What it models | Expected |
|---|---|---|
| `01-dmarc-fail` | the account's server recorded `dmarc=fail` | notice: DMARC |
| `02-forged-lower-hop` | the server's topmost header passes; a lower `dmarc=fail` with the same authserv-id and one from another server were written by the sender | no notice |
| `03-display-name-spoof` | `"paypal.com Security" <alerts@mailer.example.net>` | notice: the name shows paypal.com |
| `04-punycode-sender` | an all-Cyrillic `xn--` sender domain | notice: the decoded name |
| `05-mixed-script-sender` | a raw UTF-8 domain with a Cyrillic `а` in a Latin name | notice: mixes Latin and Cyrillic |
| `06-smime-enveloped` | `application/pkcs7-mime; smime-type=enveloped-data` | encrypted (S/MIME), `smime.p7m` to save |
| `07-pgp-mime-encrypted` | `multipart/encrypted` (RFC 3156) | encrypted (PGP), `encrypted.asc` to save |
| `08-pgp-inline` | a `-----BEGIN PGP MESSAGE-----` block in plain text | encrypted (PGP), `message.asc` to save |
| `09-unsubscribe-mailto` | `List-Unsubscribe` with https, then mailto with a subject | Unsubscribe opens the composer |
| `10-unsubscribe-https` | `List-Unsubscribe` with plain http, then https, and One-Click | Unsubscribe asks before the browser (https) |

Keep CRLF line endings when editing them (`.gitattributes`).
