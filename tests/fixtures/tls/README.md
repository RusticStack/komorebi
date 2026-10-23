# Test TLS identity

`test-cert.pem` and `test-key.pem` are a throwaway self-signed certificate
for `127.0.0.1` / `localhost`, used only by the fake HTTPS server in the
`fetch` unit tests. They protect nothing and are trusted nowhere else.

`other-cert.pem` and `other-key.pem` are a second throwaway identity with the
same names (`127.0.0.1` / `localhost`), used by the certificate pinning tests
(`src/mail/pin.rs`) as the "changed certificate". Equally worthless and
trusted nowhere.

`renewed-cert.pem` is a renewal of `test-cert.pem` (same key, `test-key.pem`),
for the server key pin.

`ca-cert.pem` is a throwaway test CA ("Komorebi Test CA", its key was not
kept). `ca-leaf-cert.pem` (key `other-key.pem`) and `ca-leaf2-cert.pem` (key
`test-key.pem`) are two certificates it issued for `127.0.0.1` /
`localhost`, used by the CA pin tests and by the certificate review: a fake
server presenting `ca-leaf-cert.pem` with or without the CA beside it
(`src/mail/pin.rs`), and the `account-cert-ca` snapshot, which shows the
prompt offering the CA. None of them is trusted anywhere.
