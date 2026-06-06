package mail

import (
	"context"
	"encoding/xml"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"sort"
	"strconv"
	"strings"
	"time"
)

const discoveryBodyLimit = 512 * 1024

type discoveryResolver interface {
	LookupSRV(ctx context.Context, service, proto, name string) (string, []*net.SRV, error)
	LookupMX(ctx context.Context, name string) ([]*net.MX, error)
}

type discoveryHTTPClient interface {
	Do(req *http.Request) (*http.Response, error)
}

type netDiscoveryResolver struct{}

func (netDiscoveryResolver) LookupSRV(ctx context.Context, service, proto, name string) (string, []*net.SRV, error) {
	return net.DefaultResolver.LookupSRV(ctx, service, proto, name)
}

func (netDiscoveryResolver) LookupMX(ctx context.Context, name string) ([]*net.MX, error) {
	return net.DefaultResolver.LookupMX(ctx, name)
}

func (s *Service) DiscoverAccountSettings(req DiscoverAccountSettingsRequest) (DiscoverAccountSettingsResponse, error) {
	client := s.httpClient
	if client == nil {
		client = &http.Client{Timeout: 8 * time.Second}
	}
	ctx, cancel := context.WithTimeout(context.Background(), 12*time.Second)
	defer cancel()
	return discoverAccountSettings(ctx, req.Email, netDiscoveryResolver{}, client)
}

func discoverAccountSettings(ctx context.Context, rawEmail string, resolver discoveryResolver, httpClient discoveryHTTPClient) (DiscoverAccountSettingsResponse, error) {
	email, err := validateEmail(rawEmail)
	if err != nil {
		return DiscoverAccountSettingsResponse{}, err
	}
	domain := domainFromEmail(email)
	if domain == "" {
		return DiscoverAccountSettingsResponse{}, fmt.Errorf("email domain is invalid")
	}

	response := DiscoverAccountSettingsResponse{Email: email, Domain: domain}
	collector := discoveryCollector{email: email, domain: domain}

	collector.addAutoconfigCandidates(ctx, resolver, httpClient)
	collector.addSRVCandidates(ctx, resolver)
	collector.addMXDiagnostics(ctx, resolver)
	collector.addGenericFallbacks()

	response.Candidates = collector.sorted()
	response.Diagnostics = collector.diagnostics
	return response, nil
}

type discoveryCollector struct {
	email       string
	domain      string
	candidates  []DiscoveredAccountSettings
	diagnostics []DiscoveryDiagnostic
}

func (c *discoveryCollector) addAutoconfigCandidates(ctx context.Context, _ discoveryResolver, httpClient discoveryHTTPClient) {
	if httpClient == nil {
		httpClient = &http.Client{Timeout: 8 * time.Second}
	}
	escapedEmail := url.QueryEscape(c.email)
	urls := []string{
		fmt.Sprintf("https://autoconfig.%s/mail/config-v1.1.xml?emailaddress=%s", c.domain, escapedEmail),
		fmt.Sprintf("https://%s/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress=%s", c.domain, escapedEmail),
	}

	for _, endpoint := range urls {
		req, err := http.NewRequestWithContext(ctx, http.MethodGet, endpoint, nil)
		if err != nil {
			c.addDiagnostic("autoconfig", "warn", err.Error())
			continue
		}
		req.Header.Set("Accept", "application/xml,text/xml")
		resp, err := httpClient.Do(req)
		if err != nil {
			c.addDiagnostic("autoconfig", "miss", hostForDiagnostic(endpoint)+": request failed")
			continue
		}
		body, readErr := io.ReadAll(io.LimitReader(resp.Body, discoveryBodyLimit))
		closeErr := resp.Body.Close()
		if readErr != nil {
			c.addDiagnostic("autoconfig", "warn", readErr.Error())
			continue
		}
		if closeErr != nil {
			c.addDiagnostic("autoconfig", "warn", closeErr.Error())
			continue
		}
		if resp.StatusCode < 200 || resp.StatusCode >= 300 {
			c.addDiagnostic("autoconfig", "miss", fmt.Sprintf("%s returned HTTP %d", hostForDiagnostic(endpoint), resp.StatusCode))
			continue
		}
		candidates, err := parseAutoconfig(body, c.email, c.domain)
		if err != nil {
			c.addDiagnostic("autoconfig", "warn", err.Error())
			continue
		}
		for _, candidate := range candidates {
			c.addCandidate(candidate)
		}
		c.addDiagnostic("autoconfig", "pass", hostForDiagnostic(endpoint))
	}
}

func (c *discoveryCollector) addSRVCandidates(ctx context.Context, resolver discoveryResolver) {
	if resolver == nil {
		resolver = netDiscoveryResolver{}
	}

	incoming := firstSRV(ctx, resolver, "imaps", c.domain)
	incomingSecurity := "tls"
	if incoming.Host == "" {
		incoming = firstSRV(ctx, resolver, "imap", c.domain)
		incomingSecurity = "starttls"
	}
	outgoing := firstSRV(ctx, resolver, "submissions", c.domain)
	outgoingSecurity := "tls"
	if outgoing.Host == "" {
		outgoing = firstSRV(ctx, resolver, "submission", c.domain)
		outgoingSecurity = "starttls"
	}

	switch {
	case incoming.Host != "" && outgoing.Host != "":
		c.addCandidate(DiscoveredAccountSettings{
			ProviderKind:  providerGeneric,
			ProviderLabel: "DNS SRV mail",
			Source:        "dns-srv",
			Email:         c.email,
			Domain:        c.domain,
			AuthMode:      authModePassword,
			IMAPHost:      incoming.Host,
			IMAPPort:      incoming.Port,
			IMAPSecurity:  incomingSecurity,
			SMTPHost:      outgoing.Host,
			SMTPPort:      outgoing.Port,
			SMTPSecurity:  outgoingSecurity,
			Confidence:    98,
		})
		c.addDiagnostic("dns-srv", "pass", "IMAP and SMTP SRV records found")
	case incoming.Host != "":
		c.addCandidate(DiscoveredAccountSettings{
			ProviderKind:  providerGeneric,
			ProviderLabel: "DNS SRV mail",
			Source:        "dns-srv",
			Email:         c.email,
			Domain:        c.domain,
			AuthMode:      authModePassword,
			IMAPHost:      incoming.Host,
			IMAPPort:      incoming.Port,
			IMAPSecurity:  incomingSecurity,
			SMTPHost:      "smtp." + c.domain,
			SMTPPort:      587,
			SMTPSecurity:  "starttls",
			Confidence:    78,
			Warnings:      []string{"No SMTP SRV record was found; SMTP host is a conservative fallback."},
		})
		c.addDiagnostic("dns-srv", "warn", "IMAP SRV found; SMTP SRV missing")
	default:
		c.addDiagnostic("dns-srv", "miss", "No encrypted IMAP SRV records found")
	}
}

func (c *discoveryCollector) addMXDiagnostics(ctx context.Context, resolver discoveryResolver) {
	if resolver == nil {
		resolver = netDiscoveryResolver{}
	}
	mx, err := resolver.LookupMX(ctx, c.domain)
	if err != nil || len(mx) == 0 {
		if err != nil {
			c.addDiagnostic("mx", "miss", err.Error())
		} else {
			c.addDiagnostic("mx", "miss", "No MX records found")
		}
		return
	}

	var hosts []string
	for _, record := range mx {
		host := normalizeHost(record.Host)
		if host != "" {
			hosts = append(hosts, host)
		}
	}
	if len(hosts) == 0 {
		c.addDiagnostic("mx", "miss", "MX records were empty")
		return
	}
	c.addDiagnostic("mx", "pass", "MX records found; not used for IMAP/SMTP settings")
}

func (c *discoveryCollector) addGenericFallbacks() {
	c.addCandidate(DiscoveredAccountSettings{
		ProviderKind:  providerGeneric,
		ProviderLabel: "Common hosting pattern",
		Source:        "fallback",
		Email:         c.email,
		Domain:        c.domain,
		AuthMode:      authModePassword,
		IMAPHost:      "mail." + c.domain,
		IMAPPort:      993,
		IMAPSecurity:  "tls",
		SMTPHost:      "mail." + c.domain,
		SMTPPort:      465,
		SMTPSecurity:  "tls",
		Confidence:    48,
		Warnings:      []string{"Fallback only; confirm these hostnames with the provider before saving."},
	})
	c.addCandidate(DiscoveredAccountSettings{
		ProviderKind:  providerGeneric,
		ProviderLabel: "Common IMAP/SMTP pattern",
		Source:        "fallback",
		Email:         c.email,
		Domain:        c.domain,
		AuthMode:      authModePassword,
		IMAPHost:      "imap." + c.domain,
		IMAPPort:      993,
		IMAPSecurity:  "tls",
		SMTPHost:      "smtp." + c.domain,
		SMTPPort:      587,
		SMTPSecurity:  "starttls",
		Confidence:    42,
		Warnings:      []string{"Fallback only; confirm these hostnames with the provider before saving."},
	})
}

func (c *discoveryCollector) addCandidate(candidate DiscoveredAccountSettings) {
	if candidate.ProviderKind == "" || candidate.AuthMode == "" || candidate.IMAPHost == "" || candidate.SMTPHost == "" {
		return
	}
	if candidate.IMAPPort == 0 || candidate.SMTPPort == 0 {
		return
	}
	if _, err := validateHost(candidate.IMAPHost); err != nil {
		return
	}
	if _, err := validateHost(candidate.SMTPHost); err != nil {
		return
	}
	if _, err := validateSecurity(candidate.IMAPSecurity); err != nil {
		return
	}
	if _, err := validateSecurity(candidate.SMTPSecurity); err != nil {
		return
	}
	if candidate.Email == "" {
		candidate.Email = c.email
	}
	if candidate.Domain == "" {
		candidate.Domain = c.domain
	}
	if candidate.ProviderLabel == "" {
		candidate.ProviderLabel = "Generic IMAP/SMTP"
	}
	c.candidates = append(c.candidates, candidate)
}

func (c *discoveryCollector) addDiagnostic(source, status, detail string) {
	c.diagnostics = append(c.diagnostics, DiscoveryDiagnostic{Source: source, Status: status, Detail: detail})
}

func (c *discoveryCollector) sorted() []DiscoveredAccountSettings {
	seen := map[string]int{}
	var deduped []DiscoveredAccountSettings
	for _, candidate := range c.candidates {
		key := strings.Join([]string{
			candidate.ProviderKind,
			candidate.AuthMode,
			candidate.IMAPHost,
			strconv.Itoa(candidate.IMAPPort),
			candidate.IMAPSecurity,
			candidate.SMTPHost,
			strconv.Itoa(candidate.SMTPPort),
			candidate.SMTPSecurity,
		}, "|")
		if idx, ok := seen[key]; ok {
			if candidate.Confidence > deduped[idx].Confidence {
				deduped[idx] = candidate
			}
			continue
		}
		seen[key] = len(deduped)
		deduped = append(deduped, candidate)
	}
	sort.SliceStable(deduped, func(i, j int) bool {
		if deduped[i].Confidence == deduped[j].Confidence {
			return deduped[i].Source < deduped[j].Source
		}
		return deduped[i].Confidence > deduped[j].Confidence
	})
	return deduped
}

type srvEndpoint struct {
	Host string
	Port int
}

func firstSRV(ctx context.Context, resolver discoveryResolver, service, domain string) srvEndpoint {
	_, records, err := resolver.LookupSRV(ctx, service, "tcp", domain)
	if err != nil || len(records) == 0 {
		return srvEndpoint{}
	}
	record := records[0]
	return srvEndpoint{Host: normalizeHost(record.Target), Port: int(record.Port)}
}

type thunderbirdClientConfig struct {
	Provider thunderbirdEmailProvider `xml:"emailProvider"`
}

type thunderbirdEmailProvider struct {
	ID               string              `xml:"id,attr"`
	DisplayName      string              `xml:"displayName"`
	DisplayShortName string              `xml:"displayShortName"`
	Incoming         []thunderbirdServer `xml:"incomingServer"`
	Outgoing         []thunderbirdServer `xml:"outgoingServer"`
}

type thunderbirdServer struct {
	Type           string `xml:"type,attr"`
	Hostname       string `xml:"hostname"`
	Port           int    `xml:"port"`
	SocketType     string `xml:"socketType"`
	Username       string `xml:"username"`
	Authentication string `xml:"authentication"`
}

func parseAutoconfig(body []byte, email, domain string) ([]DiscoveredAccountSettings, error) {
	var config thunderbirdClientConfig
	if err := xml.Unmarshal(body, &config); err != nil {
		return nil, fmt.Errorf("autoconfig XML is invalid")
	}
	provider := config.Provider
	label := strings.TrimSpace(provider.DisplayName)
	if label == "" {
		label = strings.TrimSpace(provider.DisplayShortName)
	}
	if label == "" {
		label = "Autoconfig mail"
	}

	var candidates []DiscoveredAccountSettings
	for _, incoming := range provider.Incoming {
		if strings.ToLower(strings.TrimSpace(incoming.Type)) != "imap" {
			continue
		}
		imapSecurity, ok := securityFromAutoconfigSocket(incoming.SocketType)
		if !ok {
			continue
		}
		for _, outgoing := range provider.Outgoing {
			smtpSecurity, ok := securityFromAutoconfigSocket(outgoing.SocketType)
			if !ok {
				continue
			}
			authMode := authModePassword
			if strings.Contains(strings.ToLower(incoming.Authentication+" "+outgoing.Authentication), "oauth") {
				authMode = authModeOAuth
			}
			providerKind := providerGeneric
			if authMode == authModeOAuth {
				if strings.Contains(strings.ToLower(incoming.Hostname+" "+outgoing.Hostname), "gmail") {
					providerKind = providerGmail
				}
				if strings.Contains(strings.ToLower(incoming.Hostname+" "+outgoing.Hostname), "office365") ||
					strings.Contains(strings.ToLower(incoming.Hostname+" "+outgoing.Hostname), "outlook") {
					providerKind = providerMicrosoft
				}
			}
			candidates = append(candidates, DiscoveredAccountSettings{
				ProviderKind:  providerKind,
				ProviderLabel: label,
				Source:        "autoconfig",
				Email:         email,
				Domain:        domain,
				AuthMode:      authMode,
				IMAPHost:      normalizeHost(incoming.Hostname),
				IMAPPort:      incoming.Port,
				IMAPSecurity:  imapSecurity,
				SMTPHost:      normalizeHost(outgoing.Hostname),
				SMTPPort:      outgoing.Port,
				SMTPSecurity:  smtpSecurity,
				Confidence:    97,
			})
		}
	}
	if len(candidates) == 0 {
		return nil, fmt.Errorf("autoconfig did not contain encrypted IMAP/SMTP settings")
	}
	return candidates, nil
}

func securityFromAutoconfigSocket(value string) (string, bool) {
	switch strings.ToUpper(strings.TrimSpace(value)) {
	case "SSL", "TLS", "SSL/TLS":
		return "tls", true
	case "STARTTLS":
		return "starttls", true
	default:
		return "", false
	}
}

func domainFromEmail(email string) string {
	_, domain, ok := strings.Cut(email, "@")
	if !ok {
		return ""
	}
	return strings.ToLower(strings.TrimSpace(domain))
}

func normalizeHost(host string) string {
	return strings.TrimSuffix(strings.ToLower(strings.TrimSpace(host)), ".")
}

func hostForDiagnostic(rawURL string) string {
	parsed, err := url.Parse(rawURL)
	if err != nil || parsed.Host == "" {
		return rawURL
	}
	return parsed.Host
}
