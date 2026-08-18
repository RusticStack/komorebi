package mail

import (
	"fmt"
	"strings"
)

const (
	providerLocal     = "local"
	providerGeneric   = "generic"
	providerGmail     = "gmail"
	providerMicrosoft = "microsoft"

	authModeLocal    = "local"
	authModePassword = "password"
	authModeOAuth    = "oauth"
)

type providerPolicy struct {
	ProviderKind  string
	Label         string
	AuthMode      string
	IMAPHost      string
	IMAPPort      int
	IMAPSecurity  string
	SMTPHost      string
	SMTPPort      int
	SMTPSecurity  string
	AuthEndpoint  string
	TokenEndpoint string
	Scopes        []string
	Notes         []string
}

func listProviderPolicies() []providerPolicy {
	return []providerPolicy{
		{
			ProviderKind:  providerGmail,
			Label:         "Gmail",
			AuthMode:      authModeOAuth,
			IMAPHost:      "imap.gmail.com",
			IMAPPort:      993,
			IMAPSecurity:  "tls",
			SMTPHost:      "smtp.gmail.com",
			SMTPPort:      587,
			SMTPSecurity:  "starttls",
			AuthEndpoint:  "https://accounts.google.com/o/oauth2/v2/auth",
			TokenEndpoint: "https://oauth2.googleapis.com/token",
			Scopes: []string{
				"openid",
				"email",
				"profile",
				"https://mail.google.com/",
			},
			Notes: []string{"OAuth with PKCE is required. Password and app-password account setup is refused."},
		},
		{
			ProviderKind:  providerMicrosoft,
			Label:         "Microsoft 365 / Outlook",
			AuthMode:      authModeOAuth,
			IMAPHost:      "outlook.office365.com",
			IMAPPort:      993,
			IMAPSecurity:  "tls",
			SMTPHost:      "smtp.office365.com",
			SMTPPort:      587,
			SMTPSecurity:  "starttls",
			AuthEndpoint:  "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
			TokenEndpoint: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
			Scopes: []string{
				"offline_access",
				"openid",
				"email",
				"profile",
				"https://outlook.office.com/IMAP.AccessAsUser.All",
				"https://outlook.office.com/SMTP.Send",
			},
			Notes: []string{"OAuth with PKCE is required. Password account setup is refused."},
		},
		{
			ProviderKind: providerGeneric,
			Label:        "Generic IMAP/SMTP",
			AuthMode:     authModePassword,
			IMAPSecurity: "tls",
			SMTPSecurity: "starttls",
			Notes:        []string{"Requires explicit hostnames and encrypted IMAP/SMTP settings."},
		},
		{
			ProviderKind: providerLocal,
			Label:        "Local demo cache",
			AuthMode:     authModeLocal,
			Notes:        []string{"Local-only account used for first-run demo data and offline UI testing."},
		},
	}
}

func providerPolicyByKind(kind string) (providerPolicy, error) {
	kind = normalizeProviderKind(kind)
	for _, policy := range listProviderPolicies() {
		if policy.ProviderKind == kind {
			return policy, nil
		}
	}
	return providerPolicy{}, fmt.Errorf("provider is not supported")
}

func normalizeProviderKind(kind string) string {
	switch strings.ToLower(strings.TrimSpace(kind)) {
	case providerGmail:
		return providerGmail
	case "outlook", "office365", "m365", providerMicrosoft:
		return providerMicrosoft
	case providerGeneric:
		return providerGeneric
	case providerLocal:
		return providerLocal
	default:
		return ""
	}
}

func authModeForProvider(kind string) string {
	switch normalizeProviderKind(kind) {
	case providerGmail, providerMicrosoft:
		return authModeOAuth
	case providerGeneric:
		return authModePassword
	case providerLocal:
		return authModeLocal
	default:
		return ""
	}
}

func safetyForAccount(provider, imapSecurity, smtpSecurity string) string {
	provider = normalizeProviderKind(provider)
	if provider == providerLocal {
		return "local-only"
	}
	if provider == providerGmail || provider == providerMicrosoft {
		return "oauth-required"
	}
	if imapSecurity == "tls" && (smtpSecurity == "tls" || smtpSecurity == "starttls") {
		return "encrypted"
	}
	return "blocked"
}

func providerPolicyViews() []ProviderPolicyView {
	policies := listProviderPolicies()
	views := make([]ProviderPolicyView, 0, len(policies))
	for _, policy := range policies {
		views = append(views, ProviderPolicyView{
			ProviderKind: policy.ProviderKind,
			Label:        policy.Label,
			AuthMode:     policy.AuthMode,
			IMAPHost:     policy.IMAPHost,
			IMAPPort:     policy.IMAPPort,
			IMAPSecurity: policy.IMAPSecurity,
			SMTPHost:     policy.SMTPHost,
			SMTPPort:     policy.SMTPPort,
			SMTPSecurity: policy.SMTPSecurity,
			Scopes:       append([]string(nil), policy.Scopes...),
			Notes:        append([]string(nil), policy.Notes...),
		})
	}
	return views
}
