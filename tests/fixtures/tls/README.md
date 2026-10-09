# Test TLS identity

`test-cert.pem` and `test-key.pem` are a throwaway self-signed certificate
for `127.0.0.1` / `localhost`, used only by the fake HTTPS server in the
`fetch` unit tests. They protect nothing and are trusted nowhere else.
