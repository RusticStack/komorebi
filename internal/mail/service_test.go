package mail

import (
	"bufio"
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"math/big"
	"net"
	"net/http"
	"os"
	"strings"
	"testing"
	"time"

	"komorebi/internal/credentials"
	"komorebi/internal/db"

	goimap "github.com/emersion/go-imap/v2"
	"github.com/emersion/go-imap/v2/imapserver"
	"github.com/emersion/go-imap/v2/imapserver/imapmemserver"
)

type memoryCredentialStore struct {
	values map[string]credentials.CredentialData
}

func (m *memoryCredentialStore) Save(accountID string, data credentials.CredentialData) error {
	if m.values == nil {
		m.values = map[string]credentials.CredentialData{}
	}
	m.values[accountID] = data
	return nil
}

func (m *memoryCredentialStore) Get(accountID string) (credentials.CredentialData, error) {
	if value, ok := m.values[accountID]; ok {
		return value, nil
	}
	return credentials.CredentialData{}, nil
}

func (m *memoryCredentialStore) Delete(accountID string) error {
	delete(m.values, accountID)
	return nil
}

func newTestService(t *testing.T) *Service {
	t.Helper()

	database, err := db.Open(t.TempDir())
	if err != nil {
		t.Fatalf("Open() error = %v", err)
	}
	t.Cleanup(func() { database.Close() })

	service := NewService(database)
	service.credentials = &memoryCredentialStore{values: map[string]credentials.CredentialData{}}
	service.now = func() time.Time { return time.Unix(1_800_000_000, 0) }
	service.appDataDir = t.TempDir()
	service.downloadsDir = t.TempDir()
	return service
}

func TestServiceLoadsSeededShellAndMutatesLocalCache(t *testing.T) {
	service := newTestService(t)

	shell, err := service.LoadMailShell()
	if err != nil {
		t.Fatalf("LoadMailShell() error = %v", err)
	}
	if len(shell.Accounts) != 1 {
		t.Fatalf("accounts = %d, want 1", len(shell.Accounts))
	}
	if len(shell.Folders) != 5 {
		t.Fatalf("folders = %d, want 5", len(shell.Folders))
	}
	if len(shell.Messages) == 0 {
		t.Fatal("messages = 0, want seeded messages")
	}

	messageID := shell.Messages[0].ID
	if err := service.MarkRead(MarkReadRequest{MessageID: messageID, Read: false}); err != nil {
		t.Fatalf("MarkRead(false) error = %v", err)
	}
	page, err := service.LoadMessagePage(MessagePageRequest{FolderID: "inbox", Limit: 1})
	if err != nil {
		t.Fatalf("LoadMessagePage() error = %v", err)
	}
	if len(page.Messages) != 1 {
		t.Fatalf("page length = %d, want 1", len(page.Messages))
	}

	results, err := service.SearchMail(SearchRequest{Query: "planning", Limit: 10})
	if err != nil {
		t.Fatalf("SearchMail() error = %v", err)
	}
	if len(results) == 0 {
		t.Fatal("SearchMail() returned no results for seeded term")
	}
	if _, err := service.SearchMail(SearchRequest{Query: "!!!!", Limit: 10}); err != nil {
		t.Fatalf("SearchMail(malformed) error = %v", err)
	}
}

func TestServicePersistsDraftsAndCreatesLocalSentMessage(t *testing.T) {
	service := newTestService(t)
	if _, err := service.LoadMailShell(); err != nil {
		t.Fatalf("LoadMailShell() error = %v", err)
	}

	draft := Draft{
		ID:        "draft-test",
		AccountID: "demo-local",
		Subject:   "Draft smoke",
		To:        `["recipient@example.test"]`,
		BodyHTML:  "<p>Hello</p>",
	}
	if err := service.SaveDraft(draft); err != nil {
		t.Fatalf("SaveDraft() error = %v", err)
	}

	drafts, err := service.ListDrafts()
	if err != nil {
		t.Fatalf("ListDrafts() error = %v", err)
	}
	found := false
	for _, item := range drafts {
		if item.ID == draft.ID {
			found = true
		}
	}
	if !found {
		t.Fatal("saved draft not returned by ListDrafts()")
	}

	if err := service.SendDraft(draft.ID); err != nil {
		t.Fatalf("SendDraft() error = %v", err)
	}

	var sent int
	if err := service.db.QueryRow(`SELECT COUNT(*) FROM messages WHERE id = 'sent-draft-test' AND folder_id = 'sent'`).Scan(&sent); err != nil {
		t.Fatalf("count sent message: %v", err)
	}
	if sent != 1 {
		t.Fatalf("sent rows = %d, want 1", sent)
	}
}

func TestServiceRejectsUnsafeAccountSetup(t *testing.T) {
	service := newTestService(t)

	if _, err := service.AddAccount(AddAccountRequest{
		Email:        "user@gmail.com",
		DisplayName:  "Gmail User",
		ProviderKind: "gmail",
		Password:     "not-allowed",
	}); err == nil {
		t.Fatal("AddAccount(gmail password) error = nil, want OAuth-only rejection")
	}

	if _, err := service.AddAccount(AddAccountRequest{
		Email:        "user@example.test",
		DisplayName:  "Generic User",
		ProviderKind: "generic",
		Password:     "secret",
		IMAPHost:     "https://imap.example.test",
		IMAPPort:     993,
		IMAPSecurity: "tls",
		SMTPHost:     "smtp.example.test",
		SMTPPort:     587,
		SMTPSecurity: "starttls",
	}); err == nil {
		t.Fatal("AddAccount(URL host) error = nil, want host validation failure")
	}

	if _, err := service.AddAccount(AddAccountRequest{
		Email:        "user@example.test",
		DisplayName:  "Generic User",
		ProviderKind: "generic",
		Password:     "secret",
		IMAPHost:     "imap.example.test",
		IMAPPort:     993,
		IMAPSecurity: "none",
		SMTPHost:     "smtp.example.test",
		SMTPPort:     587,
		SMTPSecurity: "starttls",
	}); err == nil {
		t.Fatal("AddAccount(none security) error = nil, want TLS validation failure")
	}
}

func TestServiceSendsThroughFakeSMTPServer(t *testing.T) {
	service := newTestService(t)
	imapPort := startFakeAccountIMAPServer(t)
	smtpPort, result := startFakeSMTPServer(t)
	service.imapTLSConfig = &tls.Config{
		InsecureSkipVerify: true,
		ServerName:         "localhost",
	}
	service.smtpTLSConfig = &tls.Config{
		InsecureSkipVerify: true,
		ServerName:         "localhost",
	}

	accountID, err := service.AddAccount(AddAccountRequest{
		Email:        "sender@example.test",
		DisplayName:  "Sender",
		ProviderKind: "generic",
		Password:     "secret",
		IMAPHost:     "localhost",
		IMAPPort:     imapPort,
		IMAPSecurity: "starttls",
		SMTPHost:     "localhost",
		SMTPPort:     smtpPort,
		SMTPSecurity: "tls",
	})
	if err != nil {
		t.Fatalf("AddAccount() error = %v", err)
	}
	validation := waitForSMTPDelivery(t, result)
	if !validation.authed || validation.data != "" {
		t.Fatalf("SMTP validation session = %+v", validation)
	}

	if err := service.SendEmail(SendEmailRequest{
		AccountID: accountID,
		From:      "sender@example.test",
		To:        []string{"recipient@example.test"},
		Subject:   "Fake SMTP smoke",
		BodyHTML:  `<p>Hello <img src="https://tracker.example/pixel.png"></p>`,
	}); err != nil {
		t.Fatalf("SendEmail() error = %v", err)
	}

	delivered := waitForSMTPDelivery(t, result)
	if !delivered.authed || !strings.Contains(delivered.envelope, "MAIL FROM") || !strings.Contains(delivered.envelope, "RCPT TO") {
		t.Fatalf("SMTP envelope/authed = %+v", delivered)
	}
	if !strings.Contains(delivered.data, "Fake SMTP smoke") || strings.Contains(delivered.data, "tracker.example") {
		t.Fatalf("SMTP data was not sanitized as expected: %s", delivered.data)
	}
}

func TestServiceValidatesAccountSetupBeforeStoringCredential(t *testing.T) {
	service := newTestService(t)
	store := service.credentials.(*memoryCredentialStore)
	imapPort := startFakeAccountIMAPServer(t)
	smtpPort, result := startFakeSMTPServer(t)
	service.imapTLSConfig = &tls.Config{InsecureSkipVerify: true, ServerName: "localhost"}
	service.smtpTLSConfig = &tls.Config{InsecureSkipVerify: true, ServerName: "localhost"}

	response, err := service.ValidateAccountSetup(AccountSetupValidationRequest{
		Email:        "sender@example.test",
		DisplayName:  "Sender",
		ProviderKind: "generic",
		AuthMode:     "password",
		Password:     "secret",
		IMAPHost:     "localhost",
		IMAPPort:     imapPort,
		IMAPSecurity: "starttls",
		SMTPHost:     "localhost",
		SMTPPort:     smtpPort,
		SMTPSecurity: "tls",
	})
	if err != nil {
		t.Fatalf("ValidateAccountSetup() error = %v", err)
	}
	if !response.OK || len(response.Checks) != 2 {
		t.Fatalf("validation response = %+v", response)
	}
	validation := waitForSMTPDelivery(t, result)
	if !validation.authed || validation.data != "" {
		t.Fatalf("SMTP validation session = %+v", validation)
	}
	if len(store.values) != 0 {
		t.Fatalf("validation stored credentials: %+v", store.values)
	}
}

func TestServiceRunsAndStopsBackgroundSync(t *testing.T) {
	service := newTestService(t)
	imapPort := startFakeAccountIMAPServer(t)
	smtpPort, result := startFakeSMTPServer(t)
	service.imapTLSConfig = &tls.Config{InsecureSkipVerify: true, ServerName: "localhost"}
	service.smtpTLSConfig = &tls.Config{InsecureSkipVerify: true, ServerName: "localhost"}

	accountID, err := service.AddAccount(AddAccountRequest{
		Email:        "sender@example.test",
		DisplayName:  "Sender",
		ProviderKind: "generic",
		Password:     "secret",
		IMAPHost:     "localhost",
		IMAPPort:     imapPort,
		IMAPSecurity: "starttls",
		SMTPHost:     "localhost",
		SMTPPort:     smtpPort,
		SMTPSecurity: "tls",
	})
	if err != nil {
		t.Fatalf("AddAccount() error = %v", err)
	}
	_ = waitForSMTPDelivery(t, result)

	status, err := service.StartBackgroundSync(BackgroundSyncRequest{AccountID: accountID, IntervalSeconds: 1})
	if err != nil {
		t.Fatalf("StartBackgroundSync() error = %v", err)
	}
	t.Cleanup(func() { _ = service.StopBackgroundSync(SyncAccountRequest{AccountID: accountID}) })
	if !status.Running || status.IntervalSeconds != 300 {
		t.Fatalf("background status = %+v, want running with minimum interval", status)
	}

	var statuses []BackgroundSyncStatus
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		statuses, err = service.GetBackgroundSyncStatus()
		if err != nil {
			t.Fatalf("GetBackgroundSyncStatus() error = %v", err)
		}
		if len(statuses) == 1 && statuses[0].LastRun > 0 && statuses[0].LastError == "" {
			break
		}
		time.Sleep(50 * time.Millisecond)
	}
	if len(statuses) != 1 || statuses[0].LastRun == 0 || statuses[0].LastError != "" {
		t.Fatalf("background sync did not record a successful run: %+v", statuses)
	}

	var stateRows int
	if err := service.db.QueryRow(`SELECT COUNT(*) FROM sync_state WHERE account_id = ? AND folder_name = 'INBOX'`, accountID).Scan(&stateRows); err != nil {
		t.Fatalf("count sync state: %v", err)
	}
	if stateRows != 1 {
		t.Fatalf("sync_state rows = %d, want 1", stateRows)
	}
	var syncedRows int
	if err := service.db.QueryRow(`SELECT COUNT(*) FROM messages WHERE account_id = ? AND id LIKE ?`, accountID, "sync-"+accountID+"-%").Scan(&syncedRows); err != nil {
		t.Fatalf("count synced messages: %v", err)
	}
	if syncedRows == 0 {
		t.Fatal("background sync did not cache any messages")
	}

	if err := service.StopBackgroundSync(SyncAccountRequest{AccountID: accountID}); err != nil {
		t.Fatalf("StopBackgroundSync() error = %v", err)
	}
	statuses, err = service.GetBackgroundSyncStatus()
	if err != nil {
		t.Fatalf("GetBackgroundSyncStatus(after stop) error = %v", err)
	}
	if len(statuses) != 0 {
		t.Fatalf("statuses after stop = %+v, want none", statuses)
	}

	var enabled int
	if err := service.db.QueryRow(`SELECT enabled FROM sync_preferences WHERE account_id = ?`, accountID).Scan(&enabled); err != nil {
		t.Fatalf("read sync preference: %v", err)
	}
	if enabled != 0 {
		t.Fatalf("sync preference enabled = %d, want 0 after stop", enabled)
	}
}

func TestServicePersistsBackgroundSyncPreference(t *testing.T) {
	service := newTestService(t)
	imapPort := startFakeAccountIMAPServer(t)
	smtpPort, result := startFakeSMTPServer(t)
	service.imapTLSConfig = &tls.Config{InsecureSkipVerify: true, ServerName: "localhost"}
	service.smtpTLSConfig = &tls.Config{InsecureSkipVerify: true, ServerName: "localhost"}

	accountID, err := service.AddAccount(AddAccountRequest{
		Email:        "sender@example.test",
		DisplayName:  "Sender",
		ProviderKind: "generic",
		Password:     "secret",
		IMAPHost:     "localhost",
		IMAPPort:     imapPort,
		IMAPSecurity: "starttls",
		SMTPHost:     "localhost",
		SMTPPort:     smtpPort,
		SMTPSecurity: "tls",
	})
	if err != nil {
		t.Fatalf("AddAccount() error = %v", err)
	}
	_ = waitForSMTPDelivery(t, result)

	if _, err := service.StartBackgroundSync(BackgroundSyncRequest{AccountID: accountID, IntervalSeconds: 120, FolderPolicy: "inbox"}); err != nil {
		t.Fatalf("StartBackgroundSync() error = %v", err)
	}
	t.Cleanup(func() { _ = service.StopBackgroundSync(SyncAccountRequest{AccountID: accountID}) })

	var enabled, interval int
	var policy string
	if err := service.db.QueryRow(`SELECT enabled, interval_seconds, folder_policy FROM sync_preferences WHERE account_id = ?`, accountID).Scan(&enabled, &interval, &policy); err != nil {
		t.Fatalf("read sync preference: %v", err)
	}
	if enabled != 1 || interval != 120 || policy != "inbox" {
		t.Fatalf("sync preference = %d/%d/%s", enabled, interval, policy)
	}
}

func TestOAuthSetupUsesPKCEAndStoresProviderAccount(t *testing.T) {
	service := newTestService(t)
	store := service.credentials.(*memoryCredentialStore)
	service.exchangeOAuthToken = func(ctx context.Context, provider providerPolicy, pending oauthPending, req OAuthCompleteRequest) (oauthTokenResponse, error) {
		if pending.CodeVerifier == "" {
			t.Fatal("code verifier was not persisted server-side")
		}
		if req.Code != "auth-code" {
			t.Fatalf("code = %q, want auth-code", req.Code)
		}
		return oauthTokenResponse{
			AccessToken:  "access-token",
			RefreshToken: "refresh-token",
			ExpiresIn:    3600,
		}, nil
	}

	start, err := service.StartOAuthAccountSetup(OAuthStartRequest{
		ProviderKind: "gmail",
		Email:        "person@gmail.com",
		DisplayName:  "Person",
		ClientID:     "client-id",
		RedirectURI:  "http://127.0.0.1:4545/oauth/callback",
	})
	if err != nil {
		t.Fatalf("StartOAuthAccountSetup() error = %v", err)
	}
	if !strings.Contains(start.AuthURL, "code_challenge=") {
		t.Fatalf("AuthURL missing PKCE challenge: %s", start.AuthURL)
	}
	if strings.Contains(start.AuthURL, "code_verifier") {
		t.Fatalf("AuthURL exposes verifier: %s", start.AuthURL)
	}

	accountID, err := service.CompleteOAuthAccountSetup(OAuthCompleteRequest{
		State:       start.State,
		Code:        "auth-code",
		ClientID:    "client-id",
		RedirectURI: "http://127.0.0.1:4545/oauth/callback",
	})
	if err != nil {
		t.Fatalf("CompleteOAuthAccountSetup() error = %v", err)
	}

	account, err := service.getAccount(accountID)
	if err != nil {
		t.Fatalf("getAccount() error = %v", err)
	}
	if account.ProviderKind != providerGmail || account.IMAPHost != "imap.gmail.com" || account.SMTPSecurity != "starttls" {
		t.Fatalf("account preset = %+v", account)
	}
	if store.values[accountID].RefreshToken != "refresh-token" || store.values[accountID].AccessToken != "access-token" {
		t.Fatalf("stored OAuth credential = %+v", store.values[accountID])
	}
}

func TestOAuthLoopbackCallbackCompletesAccountSetup(t *testing.T) {
	service := newTestService(t)
	store := service.credentials.(*memoryCredentialStore)
	service.exchangeOAuthToken = func(ctx context.Context, provider providerPolicy, pending oauthPending, req OAuthCompleteRequest) (oauthTokenResponse, error) {
		if req.Code != "loopback-code" {
			t.Fatalf("code = %q, want loopback-code", req.Code)
		}
		return oauthTokenResponse{
			AccessToken:  "loopback-access",
			RefreshToken: "loopback-refresh",
			ExpiresIn:    3600,
		}, nil
	}

	port := reserveLoopbackPort(t)
	redirectURI := fmt.Sprintf("http://127.0.0.1:%d/oauth/callback", port)
	start, err := service.StartOAuthAccountSetup(OAuthStartRequest{
		ProviderKind: "microsoft",
		Email:        "person@outlook.com",
		DisplayName:  "Person",
		ClientID:     "client-id",
		RedirectURI:  redirectURI,
	})
	if err != nil {
		t.Fatalf("StartOAuthAccountSetup() error = %v", err)
	}
	if start.CallbackMode != "loopback" {
		t.Fatalf("CallbackMode = %q, want loopback", start.CallbackMode)
	}

	client := &http.Client{Timeout: 5 * time.Second}
	callbackURL := fmt.Sprintf("%s?code=loopback-code&state=%s", redirectURI, start.State)
	resp, err := client.Get(callbackURL)
	if err != nil {
		t.Fatalf("OAuth callback GET error = %v", err)
	}
	_ = resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("callback status = %d, want 200", resp.StatusCode)
	}

	status, err := service.GetOAuthCallbackStatus(OAuthCallbackStatusRequest{State: start.State})
	if err != nil {
		t.Fatalf("GetOAuthCallbackStatus() error = %v", err)
	}
	if !status.Complete || status.AccountID == "" || status.Error != "" {
		t.Fatalf("callback status = %+v", status)
	}
	if store.values[status.AccountID].RefreshToken != "loopback-refresh" {
		t.Fatalf("stored OAuth credential = %+v", store.values[status.AccountID])
	}
}

type fakeSMTPDelivery struct {
	authed   bool
	envelope string
	data     string
}

func startFakeSMTPServer(t *testing.T) (int, <-chan fakeSMTPDelivery) {
	t.Helper()

	listener, err := tls.Listen("tcp", "localhost:0", fakeMailTLSConfig(t))
	if err != nil {
		t.Fatalf("listen fake smtp: %v", err)
	}
	results := make(chan fakeSMTPDelivery, 4)
	t.Cleanup(func() {
		_ = listener.Close()
	})

	go func() {
		for {
			conn, err := listener.Accept()
			if err != nil {
				return
			}
			go func() {
				results <- handleFakeSMTPConnection(conn)
			}()
		}
	}()

	_, portString, err := net.SplitHostPort(listener.Addr().String())
	if err != nil {
		t.Fatalf("SplitHostPort() error = %v", err)
	}
	var port int
	if _, err := fmt.Sscanf(portString, "%d", &port); err != nil {
		t.Fatalf("parse port: %v", err)
	}
	return port, results
}

func handleFakeSMTPConnection(conn net.Conn) fakeSMTPDelivery {
	defer conn.Close()
	_ = conn.SetDeadline(time.Now().Add(5 * time.Second))

	reader := bufio.NewReader(conn)
	writer := bufio.NewWriter(conn)
	writeSMTPLine := func(line string) {
		_, _ = writer.WriteString(line + "\r\n")
		_ = writer.Flush()
	}

	delivery := fakeSMTPDelivery{}
	writeSMTPLine("220 localhost ESMTP")
	for {
		line, err := reader.ReadString('\n')
		if err != nil {
			return delivery
		}
		command := strings.TrimRight(line, "\r\n")
		upper := strings.ToUpper(command)
		switch {
		case strings.HasPrefix(upper, "EHLO") || strings.HasPrefix(upper, "HELO"):
			writeSMTPLine("250-localhost")
			writeSMTPLine("250-AUTH PLAIN XOAUTH2")
			writeSMTPLine("250 OK")
		case strings.HasPrefix(upper, "AUTH "):
			delivery.authed = true
			writeSMTPLine("235 2.7.0 Authentication successful")
		case strings.HasPrefix(upper, "MAIL FROM") || strings.HasPrefix(upper, "RCPT TO"):
			delivery.envelope += command + "\n"
			writeSMTPLine("250 2.1.0 OK")
		case strings.HasPrefix(upper, "DATA"):
			writeSMTPLine("354 End data with <CR><LF>.<CR><LF>")
			var data strings.Builder
			for {
				dataLine, err := reader.ReadString('\n')
				if err != nil {
					return delivery
				}
				if dataLine == ".\r\n" || dataLine == ".\n" {
					break
				}
				data.WriteString(dataLine)
			}
			delivery.data = data.String()
			writeSMTPLine("250 2.0.0 Queued")
		case strings.HasPrefix(upper, "QUIT"):
			writeSMTPLine("221 2.0.0 Bye")
			return delivery
		default:
			writeSMTPLine("250 OK")
		}
	}
}

func waitForSMTPDelivery(t *testing.T, result <-chan fakeSMTPDelivery) fakeSMTPDelivery {
	t.Helper()
	select {
	case delivered := <-result:
		return delivered
	case <-time.After(5 * time.Second):
		t.Fatal("fake SMTP server did not observe connection")
	}
	return fakeSMTPDelivery{}
}

func startFakeAccountIMAPServer(t *testing.T) int {
	t.Helper()

	memServer := imapmemserver.New()
	user := imapmemserver.NewUser("sender@example.test", "secret")
	if err := user.Create("INBOX", nil); err != nil {
		t.Fatalf("Create(INBOX) error = %v", err)
	}
	if _, err := user.Append("INBOX", strings.NewReader("From: Test <test@example.test>\r\nSubject: Hello\r\n\r\nBody\r\n"), &goimap.AppendOptions{}); err != nil {
		t.Fatalf("Append() error = %v", err)
	}
	memServer.AddUser(user)

	listener, err := net.Listen("tcp", "localhost:0")
	if err != nil {
		t.Fatalf("listen fake imap: %v", err)
	}
	server := imapserver.New(&imapserver.Options{
		Caps: goimap.CapSet{
			goimap.CapIMAP4rev1: {},
		},
		TLSConfig: fakeMailTLSConfig(t),
		NewSession: func(conn *imapserver.Conn) (imapserver.Session, *imapserver.GreetingData, error) {
			return memServer.NewSession(), nil, nil
		},
	})
	go func() { _ = server.Serve(listener) }()
	t.Cleanup(func() {
		_ = server.Close()
		_ = listener.Close()
	})

	_, portString, err := net.SplitHostPort(listener.Addr().String())
	if err != nil {
		t.Fatalf("SplitHostPort() error = %v", err)
	}
	var port int
	if _, err := fmt.Sscanf(portString, "%d", &port); err != nil {
		t.Fatalf("parse port: %v", err)
	}
	return port
}

func reserveLoopbackPort(t *testing.T) int {
	t.Helper()

	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("reserve loopback port: %v", err)
	}
	defer listener.Close()
	_, portString, err := net.SplitHostPort(listener.Addr().String())
	if err != nil {
		t.Fatalf("SplitHostPort() error = %v", err)
	}
	var port int
	if _, err := fmt.Sscanf(portString, "%d", &port); err != nil {
		t.Fatalf("parse port: %v", err)
	}
	return port
}

func fakeMailTLSConfig(t *testing.T) *tls.Config {
	t.Helper()

	privateKey, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatalf("GenerateKey() error = %v", err)
	}
	template := &x509.Certificate{
		SerialNumber: big.NewInt(1),
		NotBefore:    time.Now().Add(-time.Hour),
		NotAfter:     time.Now().Add(time.Hour),
		KeyUsage:     x509.KeyUsageDigitalSignature | x509.KeyUsageKeyEncipherment,
		ExtKeyUsage:  []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		DNSNames:     []string{"localhost"},
		IPAddresses:  []net.IP{net.ParseIP("127.0.0.1"), net.ParseIP("::1")},
	}
	der, err := x509.CreateCertificate(rand.Reader, template, template, &privateKey.PublicKey, privateKey)
	if err != nil {
		t.Fatalf("CreateCertificate() error = %v", err)
	}
	return &tls.Config{
		Certificates: []tls.Certificate{{
			Certificate: [][]byte{der},
			PrivateKey:  privateKey,
		}},
	}
}

func TestMessageDetailSanitizesHTMLAndBlocksRemoteContent(t *testing.T) {
	service := newTestService(t)
	if _, err := service.LoadMailShell(); err != nil {
		t.Fatalf("LoadMailShell() error = %v", err)
	}

	_, err := service.db.Exec(`
		INSERT INTO messages
			(id, account_id, folder_id, subject, sender_email, date_received, body_html, body_text)
		VALUES ('msg-html', 'demo-local', 'inbox', 'HTML', 'sender@example.test', unixepoch(),
		        '<p>Hello<script>alert(1)</script><img src="https://tracker.example/pixel.png"><a href="javascript:alert(1)">bad</a><a href="https://example.test">good</a></p>',
		        'Hello')
	`)
	if err != nil {
		t.Fatalf("insert html message: %v", err)
	}

	detail, err := service.LoadMessageDetail(MessageDetailRequest{MessageID: "msg-html"})
	if err != nil {
		t.Fatalf("LoadMessageDetail() error = %v", err)
	}
	if strings.Contains(detail.SafeHTML, "script") || strings.Contains(detail.SafeHTML, "javascript:") || strings.Contains(detail.SafeHTML, "<img") {
		t.Fatalf("unsafe HTML survived sanitization: %s", detail.SafeHTML)
	}
	if !strings.Contains(detail.SafeHTML, `href="https://example.test"`) {
		t.Fatalf("safe link missing from sanitized HTML: %s", detail.SafeHTML)
	}
	if detail.RemoteContentBlocked != 1 || !detail.UnsafeHTMLRemoved {
		t.Fatalf("remote/unsafe flags = %d/%v, want 1/true", detail.RemoteContentBlocked, detail.UnsafeHTMLRemoved)
	}
}

func TestRemoteContentPreferenceAllowsMessageImages(t *testing.T) {
	service := newTestService(t)
	if _, err := service.LoadMailShell(); err != nil {
		t.Fatalf("LoadMailShell() error = %v", err)
	}
	_, err := service.db.Exec(`
		INSERT INTO messages
			(id, account_id, folder_id, subject, sender_email, date_received, body_html)
		VALUES ('msg-remote', 'demo-local', 'inbox', 'Remote', 'sender@example.test', unixepoch(),
		        '<p>Hello<img src="https://cdn.example.test/pixel.png" alt="pixel"><script>alert(1)</script></p>')
	`)
	if err != nil {
		t.Fatalf("insert message: %v", err)
	}

	blocked, err := service.LoadMessageDetail(MessageDetailRequest{MessageID: "msg-remote"})
	if err != nil {
		t.Fatalf("LoadMessageDetail(blocked) error = %v", err)
	}
	if blocked.RemoteContentBlocked != 1 || strings.Contains(blocked.SafeHTML, "<img") {
		t.Fatalf("blocked detail = %+v html=%s", blocked, blocked.SafeHTML)
	}

	if err := service.SetRemoteContentPreference(RemoteContentPreferenceRequest{MessageID: "msg-remote", Scope: "message", Allowed: true}); err != nil {
		t.Fatalf("SetRemoteContentPreference() error = %v", err)
	}
	allowed, err := service.LoadMessageDetail(MessageDetailRequest{MessageID: "msg-remote"})
	if err != nil {
		t.Fatalf("LoadMessageDetail(allowed) error = %v", err)
	}
	if !allowed.RemoteContentAllowed || allowed.RemoteContentBlocked != 0 || !strings.Contains(allowed.SafeHTML, `<img src="https://cdn.example.test/pixel.png"`) {
		t.Fatalf("allowed detail = %+v html=%s", allowed, allowed.SafeHTML)
	}
	if strings.Contains(allowed.SafeHTML, "script") {
		t.Fatalf("script survived allowed remote sanitization: %s", allowed.SafeHTML)
	}
}

func TestAttachmentWorkflowUsesSafeAppPaths(t *testing.T) {
	service := newTestService(t)
	if _, err := service.LoadMailShell(); err != nil {
		t.Fatalf("LoadMailShell() error = %v", err)
	}

	staged, err := service.StageAttachment(StageAttachmentRequest{
		AccountID:   "demo-local",
		MessageID:   "msg-001",
		DisplayName: "notes.txt",
		Mime:        "text/plain",
		Data:        []byte("safe attachment text"),
	})
	if err != nil {
		t.Fatalf("StageAttachment() error = %v", err)
	}
	listed, err := service.ListAttachments(MessageDetailRequest{MessageID: "msg-001"})
	if err != nil {
		t.Fatalf("ListAttachments() error = %v", err)
	}
	if len(listed) != 1 || listed[0].ID != staged.ID {
		t.Fatalf("attachments = %+v", listed)
	}
	preview, err := service.PreviewAttachment(AttachmentRequest{AttachmentID: staged.ID})
	if err != nil {
		t.Fatalf("PreviewAttachment() error = %v", err)
	}
	if preview.Kind != "text" || !strings.Contains(preview.Text, "safe attachment") {
		t.Fatalf("preview = %+v", preview)
	}
	saved, err := service.SaveAttachment(AttachmentRequest{AttachmentID: staged.ID})
	if err != nil {
		t.Fatalf("SaveAttachment() error = %v", err)
	}
	if !strings.HasPrefix(saved.SavedPath, service.downloadsDir) {
		t.Fatalf("saved path %q outside downloads %q", saved.SavedPath, service.downloadsDir)
	}
	if _, err := os.Stat(saved.SavedPath); err != nil {
		t.Fatalf("saved file missing: %v", err)
	}
	if err := service.DeleteAttachment(AttachmentRequest{AttachmentID: staged.ID}); err != nil {
		t.Fatalf("DeleteAttachment() error = %v", err)
	}
	listed, err = service.ListAttachments(MessageDetailRequest{MessageID: "msg-001"})
	if err != nil {
		t.Fatalf("ListAttachments(after delete) error = %v", err)
	}
	if len(listed) != 0 {
		t.Fatalf("attachments after delete = %+v", listed)
	}
}

func TestDiagnosticsExportRedactsSensitiveData(t *testing.T) {
	service := newTestService(t)
	if _, err := service.LoadMailShell(); err != nil {
		t.Fatalf("LoadMailShell() error = %v", err)
	}
	result, err := service.ExportDiagnostics(DiagnosticsExportRequest{IncludeSchema: true})
	if err != nil {
		t.Fatalf("ExportDiagnostics() error = %v", err)
	}
	data, err := os.ReadFile(result.Path)
	if err != nil {
		t.Fatalf("read diagnostics: %v", err)
	}
	text := string(data)
	if strings.Contains(text, "you@local.test") || strings.Contains(text, service.appDataDir) || strings.Contains(text, "storage_path") {
		t.Fatalf("diagnostics was not redacted: %s", text)
	}
	if !strings.Contains(text, `"schemaVersions"`) || result.SizeBytes == 0 {
		t.Fatalf("diagnostics missing expected metadata: %+v %s", result, text)
	}
}
