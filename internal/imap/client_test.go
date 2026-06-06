package imap

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"math/big"
	"net"
	"strings"
	"testing"
	"time"

	goimap "github.com/emersion/go-imap/v2"
	"github.com/emersion/go-imap/v2/imapserver"
	"github.com/emersion/go-imap/v2/imapserver/imapmemserver"
	"github.com/emersion/go-sasl"
)

func TestClientFetchesIncrementalSummariesFromFakeIMAPServer(t *testing.T) {
	addr, uidValidity, closeServer := startFakeIMAPServer(t)
	defer closeServer()

	host, port := splitHostPort(t, addr)
	client := NewPasswordClient(host, port, "person@example.test", "secret", SecurityStartTLS)
	client.TLSConfig = &tls.Config{InsecureSkipVerify: true}

	result, err := client.FetchMailboxIncremental(t.Context(), "INBOX", SyncState{
		UIDValidity: uidValidity,
		LastUID:     2,
		Limit:       10,
	})
	if err != nil {
		t.Fatalf("FetchMailboxIncremental() error = %v", err)
	}
	if len(result.Messages) != 1 {
		t.Fatalf("messages = %d, want 1", len(result.Messages))
	}
	if result.Messages[0].UID != 3 || result.Messages[0].Subject != "new" {
		t.Fatalf("message = %+v", result.Messages[0])
	}
	if result.State.UIDValidity != uidValidity || result.State.LastUID != 3 {
		t.Fatalf("state = %+v", result.State)
	}
}

func TestClientAuthenticatesWithOAuthBearerFakeIMAPServer(t *testing.T) {
	addr, closeServer := startFakeOAuthIMAPServer(t, []string{sasl.OAuthBearer})
	defer closeServer()

	host, port := splitHostPort(t, addr)
	client := NewOAuthClient(host, port, "person@example.test", "access-token", SecurityStartTLS)
	client.TLSConfig = &tls.Config{InsecureSkipVerify: true}

	if err := client.TestConnection(); err != nil {
		t.Fatalf("TestConnection() error = %v", err)
	}
}

func TestClientAuthenticatesWithXOAUTH2FakeIMAPServer(t *testing.T) {
	addr, closeServer := startFakeOAuthIMAPServer(t, []string{"XOAUTH2"})
	defer closeServer()

	host, port := splitHostPort(t, addr)
	client := NewOAuthClient(host, port, "person@example.test", "access-token", SecurityStartTLS)
	client.TLSConfig = &tls.Config{InsecureSkipVerify: true}

	if err := client.TestConnection(); err != nil {
		t.Fatalf("TestConnection() error = %v", err)
	}
}

func startFakeIMAPServer(t *testing.T) (string, uint32, func()) {
	t.Helper()

	memServer := imapmemserver.New()
	user := imapmemserver.NewUser("person@example.test", "secret")
	if err := user.Create("INBOX", nil); err != nil {
		t.Fatalf("Create(INBOX) error = %v", err)
	}

	messages := []struct {
		subject string
		date    time.Time
	}{
		{subject: "old", date: time.Unix(1700000000, 0).UTC()},
		{subject: "seen", date: time.Unix(1700000100, 0).UTC()},
		{subject: "new", date: time.Unix(1700000200, 0).UTC()},
	}
	var uidValidity uint32
	for i, message := range messages {
		appendData, err := user.Append("INBOX", strings.NewReader(rawTestMessage(i+1, message.subject, message.date)), &goimap.AppendOptions{
			Time: message.date,
		})
		if err != nil {
			t.Fatalf("Append() error = %v", err)
		}
		uidValidity = appendData.UIDValidity
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
		TLSConfig: fakeLocalhostTLSConfig(t),
		Logger:    testLogger{t: t},
		NewSession: func(conn *imapserver.Conn) (imapserver.Session, *imapserver.GreetingData, error) {
			return memServer.NewSession(), nil, nil
		},
	})
	go func() { _ = server.Serve(listener) }()
	return listener.Addr().String(), uidValidity, func() {
		_ = server.Close()
		_ = listener.Close()
	}
}

func startFakeOAuthIMAPServer(t *testing.T, mechanisms []string) (string, func()) {
	t.Helper()

	memServer := imapmemserver.New()
	user := imapmemserver.NewUser("person@example.test", "unused-password")
	if err := user.Create("INBOX", nil); err != nil {
		t.Fatalf("Create(INBOX) error = %v", err)
	}
	memServer.AddUser(user)

	listener, err := net.Listen("tcp", "localhost:0")
	if err != nil {
		t.Fatalf("listen fake oauth imap: %v", err)
	}
	server := imapserver.New(&imapserver.Options{
		Caps: goimap.CapSet{
			goimap.CapIMAP4rev1: {},
		},
		TLSConfig: fakeLocalhostTLSConfig(t),
		Logger:    testLogger{t: t},
		NewSession: func(conn *imapserver.Conn) (imapserver.Session, *imapserver.GreetingData, error) {
			return &oauthTestSession{
				Session:    memServer.NewSession(),
				mechanisms: mechanisms,
				username:   "person@example.test",
				token:      "access-token",
			}, nil, nil
		},
	})
	go func() { _ = server.Serve(listener) }()
	return listener.Addr().String(), func() {
		_ = server.Close()
		_ = listener.Close()
	}
}

type oauthTestSession struct {
	imapserver.Session
	mechanisms []string
	username   string
	token      string
}

func (s *oauthTestSession) AuthenticateMechanisms() []string {
	return append([]string(nil), s.mechanisms...)
}

func (s *oauthTestSession) Authenticate(mech string) (sasl.Server, error) {
	switch strings.ToUpper(mech) {
	case sasl.OAuthBearer:
		return sasl.NewOAuthBearerServer(func(opts sasl.OAuthBearerOptions) *sasl.OAuthBearerError {
			if opts.Username != s.username || opts.Token != s.token {
				return &sasl.OAuthBearerError{Status: "invalid_token"}
			}
			if err := s.Session.Login(s.username, "unused-password"); err != nil {
				return &sasl.OAuthBearerError{Status: "invalid_token"}
			}
			return nil
		}), nil
	case "XOAUTH2":
		return xoauth2TestServer{username: s.username, token: s.token, session: s.Session}, nil
	default:
		return nil, imapserver.ErrAuthFailed
	}
}

type xoauth2TestServer struct {
	username string
	token    string
	session  imapserver.Session
}

func (s xoauth2TestServer) Next(response []byte) ([]byte, bool, error) {
	fields := strings.Split(string(response), "\x01")
	values := map[string]string{}
	for _, field := range fields {
		if key, value, ok := strings.Cut(field, "="); ok {
			values[key] = value
		}
	}
	if values["user"] != s.username || values["auth"] != "Bearer "+s.token {
		return nil, false, imapserver.ErrAuthFailed
	}
	if err := s.session.Login(s.username, "unused-password"); err != nil {
		return nil, false, imapserver.ErrAuthFailed
	}
	return nil, true, nil
}

type testLogger struct {
	t *testing.T
}

func (l testLogger) Printf(format string, args ...interface{}) {
	l.t.Logf(format, args...)
}

func rawTestMessage(id int, subject string, date time.Time) string {
	return fmt.Sprintf("From: Sender <sender@example.test>\r\nTo: Person <person@example.test>\r\nSubject: %s\r\nDate: %s\r\nMessage-ID: <%d@example.test>\r\n\r\nBody\r\n",
		subject,
		date.Format(time.RFC1123Z),
		id,
	)
}

func splitHostPort(t *testing.T, addr string) (string, int) {
	t.Helper()
	host, portString, err := net.SplitHostPort(addr)
	if err != nil {
		t.Fatalf("SplitHostPort() error = %v", err)
	}
	var port int
	if _, err := fmt.Sscanf(portString, "%d", &port); err != nil {
		t.Fatalf("parse port: %v", err)
	}
	return host, port
}

func fakeLocalhostTLSConfig(t *testing.T) *tls.Config {
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
