# Signed-mail fixtures

Two synthetic messages for the attachment list (`tests/mime_corpus.rs`):
`smime-signed.eml` (`multipart/signed` with an `application/pkcs7-signature`
part named `smime.p7s`) and `pgp-signed.eml` (`multipart/signed` with an
`application/pgp-signature` part named `signature.asc`). **None of it is real
mail**: the addresses use `example.*` domains and the "signatures" are
placeholder base64 text, not real S/MIME or OpenPGP signatures.
