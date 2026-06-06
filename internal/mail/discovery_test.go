package mail

import (
	"bytes"
	"context"
	"io"
	"net"
	"net/http"
	"strings"
	"testing"
)

type fakeDiscoveryResolver struct {
	srv map[string][]*net.SRV
	mx  map[string][]*net.MX
}

func (f fakeDiscoveryResolver) LookupSRV(ctx context.Context, service, proto, name string) (string, []*net.SRV, error) {
	return "", f.srv[service+"."+proto+"."+name], nil
}

func (f fakeDiscoveryResolver) LookupMX(ctx context.Context, name string) ([]*net.MX, error) {
	return f.mx[name], nil
}

type roundTripFunc func(req *http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(req *http.Request) (*http.Response, error) {
	return f(req)
}

func TestDiscoveryUsesSRVForCustomDomains(t *testing.T) {
	resolver := fakeDiscoveryResolver{srv: map[string][]*net.SRV{
		"imaps.tcp.rusticstack.com": {
			{Target: "imap.postale.io.", Port: 993, Priority: 10},
		},
		"submission.tcp.rusticstack.com": {
			{Target: "smtp.postale.io.", Port: 587, Priority: 10},
		},
	}}
	client := &http.Client{Transport: roundTripFunc(func(req *http.Request) (*http.Response, error) {
		return &http.Response{StatusCode: http.StatusNotFound, Body: io.NopCloser(bytes.NewReader(nil))}, nil
	})}

	result, err := discoverAccountSettings(context.Background(), "hello@rusticstack.com", resolver, client)
	if err != nil {
		t.Fatalf("discoverAccountSettings() error = %v", err)
	}
	if len(result.Candidates) == 0 {
		t.Fatal("no candidates returned")
	}
	top := result.Candidates[0]
	if top.Source != "dns-srv" || top.IMAPHost != "imap.postale.io" || top.SMTPHost != "smtp.postale.io" || top.SMTPSecurity != "starttls" {
		t.Fatalf("top candidate = %+v", top)
	}
}

func TestDiscoveryDoesNotUseMXFingerprintsForHostedProviders(t *testing.T) {
	resolver := fakeDiscoveryResolver{mx: map[string][]*net.MX{
		"example.com": {
			{Host: "mx.zoho.eu.", Pref: 10},
		},
	}}
	client := &http.Client{Transport: roundTripFunc(func(req *http.Request) (*http.Response, error) {
		return &http.Response{StatusCode: http.StatusNotFound, Body: io.NopCloser(bytes.NewReader(nil))}, nil
	})}

	result, err := discoverAccountSettings(context.Background(), "person@example.com", resolver, client)
	if err != nil {
		t.Fatalf("discoverAccountSettings() error = %v", err)
	}
	if len(result.Candidates) == 0 {
		t.Fatal("no candidates returned")
	}
	top := result.Candidates[0]
	if top.Source == "mx" || strings.Contains(top.IMAPHost, "zoho") || strings.Contains(top.SMTPHost, "zoho") {
		t.Fatalf("MX fingerprint produced a provider-specific candidate: %+v", top)
	}
	foundMXDiagnostic := false
	for _, diagnostic := range result.Diagnostics {
		if diagnostic.Source == "mx" && diagnostic.Status == "pass" {
			foundMXDiagnostic = true
		}
	}
	if !foundMXDiagnostic {
		t.Fatalf("expected MX diagnostic, got %+v", result.Diagnostics)
	}
}

func TestDiscoveryParsesAutoconfig(t *testing.T) {
	xml := `
<clientConfig version="1.1">
  <emailProvider id="custom">
    <displayName>Custom Host</displayName>
    <incomingServer type="imap">
      <hostname>mail.provider.test</hostname>
      <port>993</port>
      <socketType>SSL</socketType>
      <username>%EMAILADDRESS%</username>
      <authentication>password-cleartext</authentication>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>submit.provider.test</hostname>
      <port>587</port>
      <socketType>STARTTLS</socketType>
      <username>%EMAILADDRESS%</username>
      <authentication>password-cleartext</authentication>
    </outgoingServer>
  </emailProvider>
</clientConfig>`
	client := &http.Client{Transport: roundTripFunc(func(req *http.Request) (*http.Response, error) {
		if strings.Contains(req.URL.Host, "autoconfig.") {
			return &http.Response{StatusCode: http.StatusOK, Body: io.NopCloser(strings.NewReader(xml))}, nil
		}
		return &http.Response{StatusCode: http.StatusNotFound, Body: io.NopCloser(bytes.NewReader(nil))}, nil
	})}

	result, err := discoverAccountSettings(context.Background(), "user@example.test", fakeDiscoveryResolver{}, client)
	if err != nil {
		t.Fatalf("discoverAccountSettings() error = %v", err)
	}
	if len(result.Candidates) == 0 {
		t.Fatal("no candidates returned")
	}
	top := result.Candidates[0]
	if top.Source != "autoconfig" || top.ProviderLabel != "Custom Host" || top.IMAPHost != "mail.provider.test" || top.SMTPHost != "submit.provider.test" {
		t.Fatalf("top candidate = %+v", top)
	}
}
