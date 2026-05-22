use crate::auth::ProviderKind;
use std::fmt;

pub const REFRESH_SKEW_SECONDS: u64 = 300;

#[derive(Clone, PartialEq, Eq)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconnectReason {
    MissingRefreshToken,
    RefreshTokenExpiredOrRevoked,
    RefreshFailed,
    CredentialStoreUnavailable,
    AccountDataUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountAuthView {
    Connected {
        access_expires_at: u64,
    },
    ReconnectNeeded {
        reason: ReconnectReason,
        provider: ProviderKind,
    },
    Removed {
        local_cache_deleted: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedAccessToken {
    token: SecretString,
    expires_at: u64,
}

impl CachedAccessToken {
    pub fn new(token: SecretString, expires_at: u64) -> Self {
        Self { token, expires_at }
    }

    pub fn token(&self) -> &SecretString {
        &self.token
    }

    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }

    fn is_fresh_at(&self, now: u64) -> bool {
        self.expires_at.saturating_sub(now) > REFRESH_SKEW_SECONDS
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthRefreshResponse {
    pub access_token: SecretString,
    pub access_expires_at: u64,
    pub replacement_refresh_token: Option<SecretString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshFailure {
    InvalidGrant,
    Network,
    ProviderUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevocationOutcome {
    Revoked,
    ProviderRevocationUnsupported,
    Failed,
    NoRefreshTokenPresent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretStoreError {
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountDataError {
    Unavailable,
}

pub trait CredentialStore {
    fn load_refresh_token(
        &mut self,
        account_id: &str,
    ) -> Result<Option<SecretString>, SecretStoreError>;

    fn store_refresh_token(
        &mut self,
        account_id: &str,
        token: SecretString,
    ) -> Result<(), SecretStoreError>;

    fn delete_account_secrets(&mut self, account_id: &str) -> Result<(), SecretStoreError>;
}

pub trait AccessTokenCache {
    fn load_access_token(&mut self, account_id: &str) -> Option<CachedAccessToken>;
    fn store_access_token(&mut self, account_id: &str, token: CachedAccessToken);
    fn clear_access_token(&mut self, account_id: &str);
}

pub trait OAuthTokenClient {
    fn refresh_access_token(
        &mut self,
        provider: ProviderKind,
        refresh_token: &SecretString,
        now: u64,
    ) -> Result<OAuthRefreshResponse, RefreshFailure>;

    fn revoke_refresh_token(
        &mut self,
        provider: ProviderKind,
        refresh_token: &SecretString,
    ) -> RevocationOutcome;
}

pub trait AccountDataStore {
    fn mark_reconnect_needed(
        &mut self,
        account_id: &str,
        reason: ReconnectReason,
    ) -> Result<(), AccountDataError>;

    fn delete_local_cache(&mut self, account_id: &str) -> Result<(), AccountDataError>;
    fn delete_account_metadata(&mut self, account_id: &str) -> Result<(), AccountDataError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenGrant {
    access_token: SecretString,
    access_expires_at: u64,
}

impl TokenGrant {
    pub fn access_token(&self) -> &SecretString {
        &self.access_token
    }

    pub fn access_expires_at(&self) -> u64 {
        self.access_expires_at
    }

    pub fn to_view(&self) -> AccountAuthView {
        AccountAuthView::Connected {
            access_expires_at: self.access_expires_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenRefreshResult {
    Ready {
        grant: TokenGrant,
        view: AccountAuthView,
    },
    ReconnectNeeded {
        view: AccountAuthView,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountRemovalPolicy {
    pub delete_local_cache: bool,
    pub attempt_provider_revocation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRemovalReport {
    pub view: AccountAuthView,
    pub provider_revocation: RevocationOutcome,
    pub credentials_deleted: bool,
    pub local_cache_deleted: bool,
    pub metadata_deleted: bool,
}

pub struct TokenLifecycle<C, A, O, D> {
    credential_store: C,
    access_cache: A,
    oauth_client: O,
    account_data: D,
}

impl<C, A, O, D> TokenLifecycle<C, A, O, D>
where
    C: CredentialStore,
    A: AccessTokenCache,
    O: OAuthTokenClient,
    D: AccountDataStore,
{
    pub fn new(credential_store: C, access_cache: A, oauth_client: O, account_data: D) -> Self {
        Self {
            credential_store,
            access_cache,
            oauth_client,
            account_data,
        }
    }

    pub fn ensure_access_token(
        &mut self,
        account_id: &str,
        provider: ProviderKind,
        now: u64,
    ) -> TokenRefreshResult {
        if let Some(cached) = self.access_cache.load_access_token(account_id) {
            if cached.is_fresh_at(now) {
                let grant = TokenGrant {
                    access_token: cached.token,
                    access_expires_at: cached.expires_at,
                };

                return TokenRefreshResult::Ready {
                    view: grant.to_view(),
                    grant,
                };
            }
        }

        let refresh_token = match self.credential_store.load_refresh_token(account_id) {
            Ok(Some(token)) => token,
            Ok(None) => {
                return self.reconnect(account_id, provider, ReconnectReason::MissingRefreshToken)
            }
            Err(SecretStoreError::Unavailable) => {
                return self.reconnect(
                    account_id,
                    provider,
                    ReconnectReason::CredentialStoreUnavailable,
                );
            }
        };

        match self
            .oauth_client
            .refresh_access_token(provider, &refresh_token, now)
        {
            Ok(response) => {
                if let Some(replacement_refresh) = response.replacement_refresh_token {
                    if self
                        .credential_store
                        .store_refresh_token(account_id, replacement_refresh)
                        .is_err()
                    {
                        self.access_cache.clear_access_token(account_id);
                        return self.reconnect(
                            account_id,
                            provider,
                            ReconnectReason::CredentialStoreUnavailable,
                        );
                    }
                }

                let cached = CachedAccessToken::new(
                    response.access_token.clone(),
                    response.access_expires_at,
                );
                self.access_cache.store_access_token(account_id, cached);

                let grant = TokenGrant {
                    access_token: response.access_token,
                    access_expires_at: response.access_expires_at,
                };

                TokenRefreshResult::Ready {
                    view: grant.to_view(),
                    grant,
                }
            }
            Err(RefreshFailure::InvalidGrant) => {
                self.access_cache.clear_access_token(account_id);
                self.reconnect(
                    account_id,
                    provider,
                    ReconnectReason::RefreshTokenExpiredOrRevoked,
                )
            }
            Err(RefreshFailure::Network | RefreshFailure::ProviderUnavailable) => {
                self.access_cache.clear_access_token(account_id);
                self.reconnect(account_id, provider, ReconnectReason::RefreshFailed)
            }
        }
    }

    pub fn remove_account(
        &mut self,
        account_id: &str,
        provider: ProviderKind,
        policy: AccountRemovalPolicy,
    ) -> AccountRemovalReport {
        let refresh_token = self
            .credential_store
            .load_refresh_token(account_id)
            .ok()
            .flatten();

        let provider_revocation = match (policy.attempt_provider_revocation, refresh_token.as_ref())
        {
            (true, Some(token)) => self.oauth_client.revoke_refresh_token(provider, token),
            (true, None) => RevocationOutcome::NoRefreshTokenPresent,
            (false, _) => RevocationOutcome::ProviderRevocationUnsupported,
        };

        self.access_cache.clear_access_token(account_id);
        let credentials_deleted = self
            .credential_store
            .delete_account_secrets(account_id)
            .is_ok();

        let local_cache_deleted = if policy.delete_local_cache {
            self.account_data.delete_local_cache(account_id).is_ok()
        } else {
            false
        };

        let metadata_deleted = self
            .account_data
            .delete_account_metadata(account_id)
            .is_ok();

        AccountRemovalReport {
            view: AccountAuthView::Removed {
                local_cache_deleted,
            },
            provider_revocation,
            credentials_deleted,
            local_cache_deleted,
            metadata_deleted,
        }
    }

    fn reconnect(
        &mut self,
        account_id: &str,
        provider: ProviderKind,
        reason: ReconnectReason,
    ) -> TokenRefreshResult {
        let final_reason = match self
            .account_data
            .mark_reconnect_needed(account_id, reason.clone())
        {
            Ok(()) => reason,
            Err(AccountDataError::Unavailable) => ReconnectReason::AccountDataUnavailable,
        };

        TokenRefreshResult::ReconnectNeeded {
            view: AccountAuthView::ReconnectNeeded {
                reason: final_reason,
                provider,
            },
        }
    }

    #[cfg(test)]
    fn into_parts(self) -> (C, A, O, D) {
        (
            self.credential_store,
            self.access_cache,
            self.oauth_client,
            self.account_data,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    const ACCOUNT_ID: &str = "account-1";
    const NOW: u64 = 1_800_000_000;

    #[derive(Default)]
    struct FakeCredentialStore {
        refresh_tokens: HashMap<String, SecretString>,
        deleted_accounts: HashSet<String>,
        fail_reads: bool,
        fail_writes: bool,
    }

    impl CredentialStore for FakeCredentialStore {
        fn load_refresh_token(
            &mut self,
            account_id: &str,
        ) -> Result<Option<SecretString>, SecretStoreError> {
            if self.fail_reads {
                return Err(SecretStoreError::Unavailable);
            }

            Ok(self.refresh_tokens.get(account_id).cloned())
        }

        fn store_refresh_token(
            &mut self,
            account_id: &str,
            token: SecretString,
        ) -> Result<(), SecretStoreError> {
            if self.fail_writes {
                return Err(SecretStoreError::Unavailable);
            }

            self.refresh_tokens.insert(account_id.to_string(), token);
            Ok(())
        }

        fn delete_account_secrets(&mut self, account_id: &str) -> Result<(), SecretStoreError> {
            self.refresh_tokens.remove(account_id);
            self.deleted_accounts.insert(account_id.to_string());
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeAccessTokenCache {
        tokens: HashMap<String, CachedAccessToken>,
        cleared_accounts: HashSet<String>,
    }

    impl AccessTokenCache for FakeAccessTokenCache {
        fn load_access_token(&mut self, account_id: &str) -> Option<CachedAccessToken> {
            self.tokens.get(account_id).cloned()
        }

        fn store_access_token(&mut self, account_id: &str, token: CachedAccessToken) {
            self.tokens.insert(account_id.to_string(), token);
        }

        fn clear_access_token(&mut self, account_id: &str) {
            self.tokens.remove(account_id);
            self.cleared_accounts.insert(account_id.to_string());
        }
    }

    struct FakeOAuthClient {
        refresh_result: Result<OAuthRefreshResponse, RefreshFailure>,
        refresh_calls: usize,
        revocation_outcome: RevocationOutcome,
        revoked_tokens: Vec<String>,
    }

    impl Default for FakeOAuthClient {
        fn default() -> Self {
            Self {
                refresh_result: Ok(OAuthRefreshResponse {
                    access_token: SecretString::new("access-secret"),
                    access_expires_at: NOW + 3600,
                    replacement_refresh_token: Some(SecretString::new("refresh-secret-2")),
                }),
                refresh_calls: 0,
                revocation_outcome: RevocationOutcome::Revoked,
                revoked_tokens: Vec::new(),
            }
        }
    }

    impl OAuthTokenClient for FakeOAuthClient {
        fn refresh_access_token(
            &mut self,
            _provider: ProviderKind,
            _refresh_token: &SecretString,
            _now: u64,
        ) -> Result<OAuthRefreshResponse, RefreshFailure> {
            self.refresh_calls += 1;
            self.refresh_result.clone()
        }

        fn revoke_refresh_token(
            &mut self,
            _provider: ProviderKind,
            refresh_token: &SecretString,
        ) -> RevocationOutcome {
            self.revoked_tokens
                .push(refresh_token.expose_secret().to_string());
            self.revocation_outcome.clone()
        }
    }

    #[derive(Default)]
    struct FakeAccountDataStore {
        reconnect_needed: Vec<(String, ReconnectReason)>,
        cache_deleted: HashSet<String>,
        metadata_deleted: HashSet<String>,
    }

    impl AccountDataStore for FakeAccountDataStore {
        fn mark_reconnect_needed(
            &mut self,
            account_id: &str,
            reason: ReconnectReason,
        ) -> Result<(), AccountDataError> {
            self.reconnect_needed.push((account_id.to_string(), reason));
            Ok(())
        }

        fn delete_local_cache(&mut self, account_id: &str) -> Result<(), AccountDataError> {
            self.cache_deleted.insert(account_id.to_string());
            Ok(())
        }

        fn delete_account_metadata(&mut self, account_id: &str) -> Result<(), AccountDataError> {
            self.metadata_deleted.insert(account_id.to_string());
            Ok(())
        }
    }

    fn lifecycle_with_refresh_token() -> TokenLifecycle<
        FakeCredentialStore,
        FakeAccessTokenCache,
        FakeOAuthClient,
        FakeAccountDataStore,
    > {
        let mut credentials = FakeCredentialStore::default();
        credentials.refresh_tokens.insert(
            ACCOUNT_ID.to_string(),
            SecretString::new("refresh-secret-1"),
        );

        TokenLifecycle::new(
            credentials,
            FakeAccessTokenCache::default(),
            FakeOAuthClient::default(),
            FakeAccountDataStore::default(),
        )
    }

    #[test]
    fn expired_access_token_refreshes_without_exposing_secret_in_view() {
        let mut lifecycle = lifecycle_with_refresh_token();

        let result = lifecycle.ensure_access_token(ACCOUNT_ID, ProviderKind::Gmail, NOW);

        let TokenRefreshResult::Ready { grant, view } = result else {
            panic!("expected refreshed token");
        };

        assert_eq!(grant.access_token().expose_secret(), "access-secret");
        assert_eq!(
            view,
            AccountAuthView::Connected {
                access_expires_at: NOW + 3600,
            },
        );
        assert!(!format!("{view:?}").contains("access-secret"));
        assert!(!format!("{view:?}").contains("refresh-secret"));

        let (credentials, access_cache, oauth, account_data) = lifecycle.into_parts();
        assert_eq!(oauth.refresh_calls, 1);
        assert_eq!(
            credentials
                .refresh_tokens
                .get(ACCOUNT_ID)
                .expect("rotated refresh token")
                .expose_secret(),
            "refresh-secret-2",
        );
        assert_eq!(
            access_cache
                .tokens
                .get(ACCOUNT_ID)
                .expect("cached access token")
                .token()
                .expose_secret(),
            "access-secret",
        );
        assert!(account_data.reconnect_needed.is_empty());
    }

    #[test]
    fn fresh_cached_access_token_skips_refresh() {
        let mut lifecycle = lifecycle_with_refresh_token();
        lifecycle.access_cache.store_access_token(
            ACCOUNT_ID,
            CachedAccessToken::new(SecretString::new("cached-access"), NOW + 1200),
        );

        let result = lifecycle.ensure_access_token(ACCOUNT_ID, ProviderKind::Microsoft, NOW);

        let TokenRefreshResult::Ready { grant, view } = result else {
            panic!("expected cached token");
        };

        assert_eq!(grant.access_token().expose_secret(), "cached-access");
        assert_eq!(
            view,
            AccountAuthView::Connected {
                access_expires_at: NOW + 1200,
            },
        );

        let (_, _, oauth, _) = lifecycle.into_parts();
        assert_eq!(oauth.refresh_calls, 0);
    }

    #[test]
    fn invalid_refresh_token_marks_reconnect_needed() {
        let mut lifecycle = lifecycle_with_refresh_token();
        lifecycle.oauth_client.refresh_result = Err(RefreshFailure::InvalidGrant);

        let result = lifecycle.ensure_access_token(ACCOUNT_ID, ProviderKind::Microsoft, NOW);

        assert_eq!(
            result,
            TokenRefreshResult::ReconnectNeeded {
                view: AccountAuthView::ReconnectNeeded {
                    reason: ReconnectReason::RefreshTokenExpiredOrRevoked,
                    provider: ProviderKind::Microsoft,
                },
            },
        );

        let (_, access_cache, _, account_data) = lifecycle.into_parts();
        assert!(access_cache.cleared_accounts.contains(ACCOUNT_ID));
        assert_eq!(
            account_data.reconnect_needed,
            vec![(
                ACCOUNT_ID.to_string(),
                ReconnectReason::RefreshTokenExpiredOrRevoked,
            )],
        );
    }

    #[test]
    fn missing_refresh_token_marks_reconnect_needed() {
        let mut lifecycle = TokenLifecycle::new(
            FakeCredentialStore::default(),
            FakeAccessTokenCache::default(),
            FakeOAuthClient::default(),
            FakeAccountDataStore::default(),
        );

        let result = lifecycle.ensure_access_token(ACCOUNT_ID, ProviderKind::Gmail, NOW);

        assert_eq!(
            result,
            TokenRefreshResult::ReconnectNeeded {
                view: AccountAuthView::ReconnectNeeded {
                    reason: ReconnectReason::MissingRefreshToken,
                    provider: ProviderKind::Gmail,
                },
            },
        );
    }

    #[test]
    fn failed_refresh_marks_safe_reconnect_state() {
        let mut lifecycle = lifecycle_with_refresh_token();
        lifecycle.oauth_client.refresh_result = Err(RefreshFailure::Network);

        let result = lifecycle.ensure_access_token(ACCOUNT_ID, ProviderKind::Gmail, NOW);

        assert_eq!(
            result,
            TokenRefreshResult::ReconnectNeeded {
                view: AccountAuthView::ReconnectNeeded {
                    reason: ReconnectReason::RefreshFailed,
                    provider: ProviderKind::Gmail,
                },
            },
        );

        let (_, access_cache, _, account_data) = lifecycle.into_parts();
        assert!(access_cache.cleared_accounts.contains(ACCOUNT_ID));
        assert_eq!(
            account_data.reconnect_needed,
            vec![(ACCOUNT_ID.to_string(), ReconnectReason::RefreshFailed)],
        );
    }

    #[test]
    fn account_removal_revokes_and_clears_local_state() {
        let mut lifecycle = lifecycle_with_refresh_token();
        lifecycle.access_cache.store_access_token(
            ACCOUNT_ID,
            CachedAccessToken::new(SecretString::new("access-secret"), NOW + 3600),
        );

        let report = lifecycle.remove_account(
            ACCOUNT_ID,
            ProviderKind::Gmail,
            AccountRemovalPolicy {
                delete_local_cache: true,
                attempt_provider_revocation: true,
            },
        );

        assert_eq!(
            report,
            AccountRemovalReport {
                view: AccountAuthView::Removed {
                    local_cache_deleted: true,
                },
                provider_revocation: RevocationOutcome::Revoked,
                credentials_deleted: true,
                local_cache_deleted: true,
                metadata_deleted: true,
            },
        );

        let (credentials, access_cache, oauth, account_data) = lifecycle.into_parts();
        assert!(credentials.deleted_accounts.contains(ACCOUNT_ID));
        assert!(!credentials.refresh_tokens.contains_key(ACCOUNT_ID));
        assert!(access_cache.cleared_accounts.contains(ACCOUNT_ID));
        assert!(!access_cache.tokens.contains_key(ACCOUNT_ID));
        assert_eq!(oauth.revoked_tokens, vec!["refresh-secret-1"]);
        assert!(account_data.cache_deleted.contains(ACCOUNT_ID));
        assert!(account_data.metadata_deleted.contains(ACCOUNT_ID));
    }

    #[test]
    fn account_removal_cleans_local_state_even_if_provider_revocation_fails() {
        let mut lifecycle = lifecycle_with_refresh_token();
        lifecycle.oauth_client.revocation_outcome = RevocationOutcome::Failed;

        let report = lifecycle.remove_account(
            ACCOUNT_ID,
            ProviderKind::Microsoft,
            AccountRemovalPolicy {
                delete_local_cache: true,
                attempt_provider_revocation: true,
            },
        );

        assert_eq!(report.provider_revocation, RevocationOutcome::Failed);
        assert!(report.credentials_deleted);
        assert!(report.local_cache_deleted);
        assert!(report.metadata_deleted);
    }
}
