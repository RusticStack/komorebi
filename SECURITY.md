# Security policy

## Supported versions

Security fixes go into the latest release. Please update before reporting.

## Reporting a vulnerability

Do not open a public issue for a security problem. Report it privately through
GitHub: open the Security tab of this repository and choose "Report a vulnerability"
(https://github.com/RusticStack/komorebi/security/advisories/new).

Include the Komorebi version, what you found, and the steps to reproduce it. Do not
include real mail, passwords or other personal data.

You can expect an acknowledgement, a fix in a new release when the report is confirmed,
and credit in the changelog if you want it. Komorebi is maintained by one person, so
replies may take a few days.

## What is in scope

Problems in Komorebi itself, for example: mail content that makes the reader load
remote content or run code, a way to leak a saved password or the cache key, a flaw in
certificate checking or pinning, or a flaw in how updates are verified (the SHA-256 and
Ed25519 signature check).

Problems in a mail provider's server, or in Windows, are out of scope.

## Releases

Releases are built from this repository on GitHub Actions. The in-app update installs
only an installer whose hash and Ed25519 signature check out against the key compiled
into the app. See [docs/RELEASING.md](docs/RELEASING.md).
