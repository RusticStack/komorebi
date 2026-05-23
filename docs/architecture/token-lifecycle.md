# Token Lifecycle

Status: accepted and implemented for the backend lifecycle, provider HTTP client layer, composer send boundary, and background sync scheduler.
Last reviewed: 2026-05-23.

## Decision

Token refresh, token revocation, reconnect-needed state, and account-removal cleanup are Rust-backend responsibilities. React receives only redacted account-auth status and must never receive access tokens, refresh tokens, authorization headers, credential target names, or provider token response bodies.

The enforceable lifecycle core lives in `src-tauri/src/token_lifecycle.rs`. It is intentionally written behind traits so Windows Credential Manager, OAuth HTTP clients, SQLite account state, composer send, and sync workers can be connected without changing frontend behavior. The Windows credential implementation lives in `src-tauri/src/credentials.rs`; the provider OAuth HTTP implementation lives in `src-tauri/src/oauth.rs`; background sync access-token refresh is orchestrated from `src-tauri/src/sync_scheduler.rs`.

## Refresh Flow

When sync/send needs an OAuth access token:

1. Rust checks the in-memory access-token cache.
2. A cached access token may be reused only when it has more than `REFRESH_SKEW_SECONDS` remaining.
3. If the cached token is missing or too close to expiry, Rust reads the refresh token from Windows Credential Manager through the credential-store abstraction.
4. Rust sends the refresh token to the provider refresh client.
5. On success, Rust stores the new access token in memory, rotates the stored refresh token when the provider returns a replacement, and returns an internal `TokenGrant` to sync/send code.
6. React receives only `AccountAuthView::Connected { access_expires_at }`.

Token secrets are redacted in debug output. Frontend-safe views do not include token fields.

## Reconnect-Needed State

Refresh failure must not leak secrets to React. The lifecycle returns a safe reconnect-needed view and records reconnect state in the account-data store.

Reconnect reasons:

- `MissingRefreshToken`: credential backend has no refresh token for the account.
- `RefreshTokenExpiredOrRevoked`: provider returns an invalid-grant style failure.
- `RefreshFailed`: provider/network refresh failed without proving revocation.
- `CredentialStoreUnavailable`: secret backend cannot be read or updated.
- `AccountDataUnavailable`: account-state persistence cannot be updated.

UI may show a reconnect prompt using these reason classes. The current account status drawer launches Gmail/Microsoft OAuth reauthentication through the backend-owned loopback flow and generic IMAP/SMTP reauthentication through a replacement-secret form. It must not show provider token responses, authorization headers, or raw OAuth error descriptions.

## Revocation And Account Removal

Account removal is a single backend workflow:

1. Stop sync workers for the account when sync exists.
2. Read the refresh token if present.
3. Attempt provider revocation when supported and requested.
4. Clear any in-memory access token.
5. Delete all credential entries for the account from Windows Credential Manager.
6. Delete local mail cache if the user chose local-cache deletion.
7. Delete account metadata.
8. Return a redacted `AccountRemovalReport`.

Local cleanup must proceed even if provider revocation fails or is unsupported. A failed provider revocation is reported as a status, not as a reason to keep local secrets or local cache.

## Provider Revocation Policy

Gmail supports programmatic revocation through Google's OAuth revocation endpoint. Komorebi's provider OAuth client posts the stored refresh token to that endpoint on account removal when revocation is requested.

Microsoft refresh tokens can expire or be revoked by the sign-in service, user actions, password changes, or admin actions. Microsoft documentation expects clients to handle revocation by reauthenticating when refresh fails. Komorebi must always delete local Microsoft tokens and cache on removal; provider-side revocation is represented as unsupported until a reviewed Microsoft-specific revocation mechanism is added.

Generic OAuth provider revocation is unsupported until provider-specific revocation endpoints are configured.

## Storage Rules

- Refresh tokens are stored only in Windows Credential Manager.
- Access tokens are memory-only by default.
- A rotated refresh token replaces the previous refresh token only after the provider refresh response succeeds.
- If refresh-token rotation cannot be stored, the access token is discarded and the account enters reconnect-needed state.
- Account removal deletes Credential Manager entries regardless of provider revocation outcome.
- SQLite account metadata stores auth status and credential references only; it never stores tokens.
- SQLite may store non-secret OAuth runtime metadata under `account:{account_id}:oauth_client` so Rust workers can reconstruct a provider refresh client. That row must contain only the OAuth client id and loopback redirect URI, never access tokens, refresh tokens, authorization codes, or token responses.

## Test Coverage

`src-tauri/src/token_lifecycle.rs` currently covers:

- Expired/missing access token refreshes using the refresh token.
- Fresh cached access token skips refresh.
- Invalid refresh token marks reconnect-needed.
- Missing refresh token marks reconnect-needed.
- Network/provider refresh failure marks reconnect-needed.
- Account removal revokes when requested and clears credentials, access cache, local cache, and metadata.
- Account removal still clears local state if provider revocation fails.

`src-tauri/src/oauth.rs` covers:

- Gmail and Microsoft OAuth authorization URLs with provider scopes and PKCE.
- Authorization-code exchange and refresh-token HTTP forms.
- Invalid-grant mapping into reconnect-needed lifecycle behavior.
- Google revocation and Microsoft revocation-unsupported behavior.
- XOAUTH2 SASL response construction without exposing tokens to React.

`src-tauri/src/credentials.rs` covers:

- Namespaced Windows Credential Manager target names.
- Account-id validation before target-name construction.
- Redacted secret debug output.

## References

- Google OAuth refresh and revocation behavior: https://developers.google.com/identity/protocols/oauth2/native-app
- Microsoft authorization-code refresh behavior: https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow
- Microsoft refresh-token lifetime and revocation behavior: https://learn.microsoft.com/en-us/entra/identity-platform/refresh-tokens
