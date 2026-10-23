# Releasing Komorebi

Two flows: the local signed flow used so far, and the future flow with code
signing by SignPath. In both, the in-app update signature (Ed25519) is made
by the maintainer on a local machine, never in CI.

## What the in-app update checks

The in-app update (Settings > About) installs only an installer whose
SHA-256 matches the published `.sha256` and whose Ed25519 signature
verifies against the public key compiled into `src/update.rs`. The
signature covers the version and the hash, so an older installer cannot be
served as a newer one. This is separate from Authenticode (the Windows
code signature): Authenticode tells Windows who published the file, the
Ed25519 signature tells Komorebi that the update came from its maintainer.

The private key is read from
`%USERPROFILE%\.komorebi-release\update-signing.key`, made once with
`cargo run --example sign_update -- keygen` (it refuses to overwrite an
existing key and prints only the public key). Keep it out of the
repository and back it up offline: losing it means shipping a new public
key by hand. It never goes to CI.

## The local signed flow (used so far)

On the maintainer's machine, from a clean checkout of the commit to
release. The Cargo version and the top `CHANGELOG.md` section already say
the new version (here `0.4.1`):

```powershell
cargo build --release --locked
pwsh scripts/build-installer.ps1 -Exe target\release\komorebi.exe -Version 0.4.1
cargo run --example sign_update -- sign dist\komorebi-0.4.1-setup.exe 0.4.1
gh release create v0.4.1 --title "Komorebi 0.4.1" --notes-file notes.md `
    dist\komorebi-0.4.1-setup.exe dist\komorebi-0.4.1-setup.exe.sha256 dist\komorebi-0.4.1-setup.exe.sig
```

1. The build writes `target\release\komorebi.exe`; the installer script
   writes `dist\komorebi-0.4.1-setup.exe` and prints its SHA-256. If an
   Authenticode certificate is available, sign `komorebi.exe` before the
   installer and the installer after it with `scripts/sign.ps1`, and only
   then sign for the update (the Ed25519 signature covers the final bytes).
2. `sign_update -- sign` writes `komorebi-0.4.1-setup.exe.sha256` and
   `.sig` beside the installer.
3. `gh release create` makes the release and its tag `v0.4.1` and uploads
   the three files the in-app update needs, under exactly these names; the
   notes are this version's `CHANGELOG.md` section. Optionally add the
   portable zip, the `.pdb` and `SHA256SUMS` built by the workflow.
4. Check it: `cargo run --example sign_update -- verify
   dist\komorebi-0.4.1-setup.exe 0.4.1`, then Settings > About > Check for
   updates on an older install.

Before any release run the checks in [BUILDING.md](BUILDING.md) (format,
clippy, tests, pins, deny, size and boot snapshot).

## The CI build (`.github/workflows/release.yml`)

It runs when a `v*` tag is pushed and by hand (Actions > Release > Run
workflow). It never publishes a release. It runs on the Blacksmith Windows
runner and does:

1. `cargo test --locked`, the release build, the size check (at most
   20,500,000 bytes) and the offline boot snapshot.
2. Uploads the unsigned `komorebi.exe` as the artifact `unsigned-exe`.
3. Builds the installer with Inno Setup (installed through Chocolatey if
   the image lacks it) and uploads it as the artifact `unsigned-installer`.
4. Builds the portable zip, the debug symbols and `SHA256SUMS`, and keeps
   everything as the workflow artifact `komorebi-<version>`.
5. On a tag (or by hand with "create_draft" ticked) attaches the packages
   to a draft release `v<version>`, with this version's `CHANGELOG.md`
   section as the notes. A tag that does not match the Cargo version fails
   the run.

The artifact names are stable so that a SignPath signing request can point
at them.

## The future flow with SignPath

Free code signing is provided by SignPath.io, certificate by SignPath
Foundation, once the application for the open-source programme is
approved. Until then the signing steps in the workflow stay commented out
and releases are unsigned.

One-time setup:

1. In SignPath, create the project and a signing policy, and connect the
   GitHub repository as a trusted build system (the artifacts must come
   from GitHub Actions in this repository).
2. Add the repository secret `SIGNPATH_API_TOKEN`.
3. In `release.yml`, uncomment the two marked SignPath steps and replace
   the placeholders `organization-id`, `project-slug` and
   `signing-policy-slug`.

Then a release goes like this:

1. Update the Cargo version and `CHANGELOG.md`, merge through a pull
   request, and push the tag `v<version>`.
2. The workflow builds, signs `komorebi.exe` through SignPath, builds the
   installer around the signed exe, and signs the installer. The signing
   requests are approved by the approver (Mario Silva) in SignPath. The
   installer wraps the exe, so the exe is signed first.
3. Download the signed installer from the draft release or the workflow
   artifacts. On the maintainer's machine make the update signature on
   those exact bytes:
   `cargo run --example sign_update -- sign komorebi-<version>-setup.exe <version>`.
4. Add the installer (signed), its `.sha256` and `.sig` to the draft
   release under exactly those names, regenerate `SHA256SUMS` for the
   files you ship, check with `sign_update -- verify`, and publish the
   draft.
5. Check Settings > About > Check for updates on an older install.

Roles in the signing policy: author and approver is Mario Silva. The
README's "Code signing policy" section and [PRIVACY.md](../PRIVACY.md) are
the public statements and must be kept in line with this page.
