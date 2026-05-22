#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Gmail,
    Microsoft,
    GenericImapSmtp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    OAuth2,
    AppPassword,
    ManualPassword,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaslMechanism {
    XOAuth2,
    Plain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthImplementation {
    OAuth2Ready,
    OAuth2Pending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthPolicy {
    Allowed {
        mechanism: SaslMechanism,
        credential_slot: &'static str,
    },
    UnsupportedUntilImplemented {
        required_auth: AuthKind,
        required_mechanism: SaslMechanism,
        reason: &'static str,
    },
    Rejected {
        reason: &'static str,
    },
}

pub fn current_auth_implementation(provider: ProviderKind) -> AuthImplementation {
    match provider {
        ProviderKind::Gmail | ProviderKind::Microsoft => AuthImplementation::OAuth2Ready,
        ProviderKind::GenericImapSmtp => AuthImplementation::OAuth2Pending,
    }
}

pub fn evaluate_account_setup_auth_policy(
    provider: ProviderKind,
    requested_auth: AuthKind,
) -> AuthPolicy {
    evaluate_auth_policy(
        provider,
        requested_auth,
        current_auth_implementation(provider),
    )
}

pub fn evaluate_auth_policy(
    provider: ProviderKind,
    requested_auth: AuthKind,
    implementation: AuthImplementation,
) -> AuthPolicy {
    match provider {
        ProviderKind::Gmail => evaluate_major_provider(
            requested_auth,
            implementation,
            "Gmail requires OAuth2/XOAUTH2 for Komorebi account setup",
        ),
        ProviderKind::Microsoft => evaluate_major_provider(
            requested_auth,
            implementation,
            "Microsoft accounts require OAuth2/XOAUTH2 for Komorebi account setup",
        ),
        ProviderKind::GenericImapSmtp => match requested_auth {
            AuthKind::AppPassword => AuthPolicy::Allowed {
                mechanism: SaslMechanism::Plain,
                credential_slot: "generic-password",
            },
            AuthKind::ManualPassword => AuthPolicy::Allowed {
                mechanism: SaslMechanism::Plain,
                credential_slot: "generic-password",
            },
            AuthKind::OAuth2 => AuthPolicy::UnsupportedUntilImplemented {
                required_auth: AuthKind::OAuth2,
                required_mechanism: SaslMechanism::XOAuth2,
                reason: "generic-provider OAuth2 requires provider-specific issuer, scopes, and token handling",
            },
        },
    }
}

fn evaluate_major_provider(
    requested_auth: AuthKind,
    implementation: AuthImplementation,
    rejection_reason: &'static str,
) -> AuthPolicy {
    match requested_auth {
        AuthKind::OAuth2 => match implementation {
            AuthImplementation::OAuth2Ready => AuthPolicy::Allowed {
                mechanism: SaslMechanism::XOAuth2,
                credential_slot: "oauth-refresh",
            },
            AuthImplementation::OAuth2Pending => AuthPolicy::UnsupportedUntilImplemented {
                required_auth: AuthKind::OAuth2,
                required_mechanism: SaslMechanism::XOAuth2,
                reason: "OAuth2/XOAUTH2 flow is unavailable in this configuration",
            },
        },
        AuthKind::AppPassword | AuthKind::ManualPassword => AuthPolicy::Rejected {
            reason: rejection_reason,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gmail_password_auth_is_rejected() {
        assert_eq!(
            evaluate_auth_policy(
                ProviderKind::Gmail,
                AuthKind::ManualPassword,
                AuthImplementation::OAuth2Pending,
            ),
            AuthPolicy::Rejected {
                reason: "Gmail requires OAuth2/XOAUTH2 for Komorebi account setup",
            },
        );
    }

    #[test]
    fn microsoft_app_password_auth_is_rejected() {
        assert_eq!(
            evaluate_auth_policy(
                ProviderKind::Microsoft,
                AuthKind::AppPassword,
                AuthImplementation::OAuth2Pending,
            ),
            AuthPolicy::Rejected {
                reason: "Microsoft accounts require OAuth2/XOAUTH2 for Komorebi account setup",
            },
        );
    }

    #[test]
    fn major_provider_oauth_is_explicitly_unsupported_while_pending() {
        assert_eq!(
            evaluate_auth_policy(
                ProviderKind::Gmail,
                AuthKind::OAuth2,
                AuthImplementation::OAuth2Pending,
            ),
            AuthPolicy::UnsupportedUntilImplemented {
                required_auth: AuthKind::OAuth2,
                required_mechanism: SaslMechanism::XOAuth2,
                reason: "OAuth2/XOAUTH2 flow is unavailable in this configuration",
            },
        );
    }

    #[test]
    fn major_provider_oauth_uses_xoauth2_when_ready() {
        assert_eq!(
            evaluate_auth_policy(
                ProviderKind::Microsoft,
                AuthKind::OAuth2,
                AuthImplementation::OAuth2Ready,
            ),
            AuthPolicy::Allowed {
                mechanism: SaslMechanism::XOAuth2,
                credential_slot: "oauth-refresh",
            },
        );
    }

    #[test]
    fn current_major_provider_oauth_is_ready() {
        assert_eq!(
            evaluate_account_setup_auth_policy(ProviderKind::Gmail, AuthKind::OAuth2),
            AuthPolicy::Allowed {
                mechanism: SaslMechanism::XOAuth2,
                credential_slot: "oauth-refresh",
            },
        );
        assert_eq!(
            evaluate_account_setup_auth_policy(ProviderKind::Microsoft, AuthKind::OAuth2),
            AuthPolicy::Allowed {
                mechanism: SaslMechanism::XOAuth2,
                credential_slot: "oauth-refresh",
            },
        );
    }

    #[test]
    fn generic_provider_password_paths_are_allowed() {
        assert_eq!(
            evaluate_auth_policy(
                ProviderKind::GenericImapSmtp,
                AuthKind::AppPassword,
                AuthImplementation::OAuth2Pending,
            ),
            AuthPolicy::Allowed {
                mechanism: SaslMechanism::Plain,
                credential_slot: "generic-password",
            },
        );
    }

    #[test]
    fn generic_oauth_requires_provider_specific_implementation() {
        assert_eq!(
            evaluate_auth_policy(
                ProviderKind::GenericImapSmtp,
                AuthKind::OAuth2,
                AuthImplementation::OAuth2Pending,
            ),
            AuthPolicy::UnsupportedUntilImplemented {
                required_auth: AuthKind::OAuth2,
                required_mechanism: SaslMechanism::XOAuth2,
                reason: "generic-provider OAuth2 requires provider-specific issuer, scopes, and token handling",
            },
        );
    }
}
