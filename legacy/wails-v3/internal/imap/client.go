package imap

import (
	"context"
	"crypto/tls"
	"fmt"
	"log"
	"net"
	"strings"
	"time"

	goimap "github.com/emersion/go-imap/v2"
	"github.com/emersion/go-imap/v2/imapclient"
	"github.com/emersion/go-sasl"
)

const (
	AuthModePassword = "password"
	AuthModeOAuth    = "oauth"

	SecurityTLS      = "tls"
	SecurityStartTLS = "starttls"

	defaultTimeout   = 30 * time.Second
	defaultSyncLimit = 50
)

// Client is a provider-safe IMAP adapter. It only connects through encrypted
// channels and supports both password auth and OAuth SASL.
type Client struct {
	Host      string
	Port      int
	Username  string
	Secret    string
	Security  string
	AuthMode  string
	Timeout   time.Duration
	TLSConfig *tls.Config
}

type MessageSummary struct {
	UID     uint32
	Subject string
	From    string
	Date    time.Time
}

type MailboxInfo struct {
	Name      string
	Delimiter string
	Attrs     []string
}

type SyncState struct {
	UIDValidity   uint32
	LastUID       uint32
	HighestModSeq uint64
	Limit         int
}

type SyncResult struct {
	Messages []MessageSummary
	State    SyncState
}

func NewPasswordClient(host string, port int, username, password, security string) *Client {
	return &Client{Host: host, Port: port, Username: username, Secret: password, Security: security, AuthMode: AuthModePassword}
}

func NewOAuthClient(host string, port int, username, accessToken, security string) *Client {
	return &Client{Host: host, Port: port, Username: username, Secret: accessToken, Security: security, AuthMode: AuthModeOAuth}
}

func (c *Client) TestConnection() error {
	client, err := c.connectAuthenticated(context.Background())
	if err != nil {
		return err
	}
	defer client.Close()
	if err := client.Noop().Wait(); err != nil {
		return fmt.Errorf("imap noop: %w", err)
	}
	_ = client.Logout().Wait()
	log.Println("IMAP connection OK for", c.Username)
	return nil
}

func (c *Client) FetchInboxSample(limit int) ([]map[string]string, error) {
	summaries, err := c.fetchMailboxSummaries(context.Background(), "INBOX", limit)
	if err != nil {
		return nil, err
	}

	results := make([]map[string]string, 0, len(summaries))
	for _, msg := range summaries {
		results = append(results, map[string]string{
			"uid":     fmt.Sprintf("%d", msg.UID),
			"subject": msg.Subject,
			"from":    msg.From,
			"date":    msg.Date.Format(time.RFC3339),
		})
	}
	return results, nil
}

func (c *Client) ListMailboxes(ctx context.Context) ([]MailboxInfo, error) {
	client, err := c.connectAuthenticated(ctx)
	if err != nil {
		return nil, err
	}
	defer client.Close()
	defer func() { _ = client.Logout().Wait() }()

	listed, err := client.List("", "*", nil).Collect()
	if err != nil {
		return nil, fmt.Errorf("list mailboxes: %w", err)
	}
	mailboxes := make([]MailboxInfo, 0, len(listed))
	for _, item := range listed {
		info := MailboxInfo{
			Name:      item.Mailbox,
			Delimiter: string(item.Delim),
			Attrs:     make([]string, 0, len(item.Attrs)),
		}
		for _, attr := range item.Attrs {
			info.Attrs = append(info.Attrs, string(attr))
		}
		mailboxes = append(mailboxes, info)
	}
	return mailboxes, nil
}

// SyncMailbox performs a bounded metadata sync. Body and attachment downloads
// remain separate so provider sync cannot unexpectedly cache large payloads.
func (c *Client) SyncMailbox(accountID, folder string, dbUpsert func(msg map[string]interface{}) error) error {
	result, err := c.FetchMailboxIncremental(context.Background(), folder, SyncState{Limit: defaultSyncLimit})
	if err != nil {
		return err
	}
	for _, msg := range result.Messages {
		if err := dbUpsert(map[string]interface{}{
			"uid":     msg.UID,
			"subject": msg.Subject,
			"from":    msg.From,
			"date":    msg.Date,
		}); err != nil {
			log.Printf("imap upsert error for %s/%s/%d: %v", accountID, folder, msg.UID, err)
		}
	}
	log.Printf("IMAP sync complete for %s/%s: %d message summaries", accountID, folder, len(result.Messages))
	return nil
}

func (c *Client) StartRealTimeSync(accountID, folder string, onUpdate func(eventType string, data interface{})) error {
	client, err := c.connectAuthenticated(context.Background())
	if err != nil {
		return err
	}
	defer client.Close()

	if _, err := client.Select(folder, nil).Wait(); err != nil {
		return fmt.Errorf("select %s: %w", folder, err)
	}
	if onUpdate != nil {
		onUpdate("idle-started", map[string]string{"accountId": accountID, "folder": folder})
	}
	idle, err := client.Idle()
	if err != nil {
		return fmt.Errorf("idle %s: %w", folder, err)
	}
	defer idle.Close()
	return idle.Wait()
}

func (c *Client) fetchMailboxSummaries(ctx context.Context, folder string, limit int) ([]MessageSummary, error) {
	result, err := c.FetchMailboxIncremental(ctx, folder, SyncState{Limit: limit})
	if err != nil {
		return nil, err
	}
	return result.Messages, nil
}

func (c *Client) FetchMailboxIncremental(ctx context.Context, folder string, state SyncState) (SyncResult, error) {
	client, err := c.connectAuthenticated(ctx)
	if err != nil {
		return SyncResult{}, err
	}
	defer client.Close()
	defer func() { _ = client.Logout().Wait() }()

	selected, err := client.Select(folder, &goimap.SelectOptions{ReadOnly: true}).Wait()
	if err != nil {
		return SyncResult{}, fmt.Errorf("select %s: %w", folder, err)
	}
	nextState := SyncState{
		UIDValidity:   selected.UIDValidity,
		LastUID:       state.LastUID,
		HighestModSeq: selected.HighestModSeq,
		Limit:         state.Limit,
	}
	if selected.NumMessages == 0 {
		nextState.LastUID = 0
		return SyncResult{Messages: []MessageSummary{}, State: nextState}, nil
	}

	limit := state.Limit
	if limit <= 0 || limit > defaultSyncLimit {
		limit = defaultSyncLimit
	}
	nextState.Limit = limit

	var numSet goimap.NumSet
	if state.UIDValidity != 0 && state.UIDValidity == selected.UIDValidity && state.LastUID != 0 {
		if selected.UIDNext <= goimap.UID(state.LastUID+1) {
			return SyncResult{Messages: []MessageSummary{}, State: nextState}, nil
		}
		startUID := goimap.UID(state.LastUID + 1)
		stopUID := selected.UIDNext - 1
		if uint32(stopUID-startUID+1) > uint32(limit) {
			startUID = stopUID - goimap.UID(limit) + 1
		}
		var uidSet goimap.UIDSet
		uidSet.AddRange(startUID, stopUID)
		numSet = uidSet
	} else {
		stop := selected.NumMessages
		start := uint32(1)
		if stop > uint32(limit) {
			start = stop - uint32(limit) + 1
		}
		var seqSet goimap.SeqSet
		seqSet.AddRange(start, stop)
		numSet = seqSet
	}

	cmd := client.Fetch(numSet, &goimap.FetchOptions{
		UID:          true,
		Envelope:     true,
		InternalDate: true,
	})

	var summaries []MessageSummary
	for {
		msg := cmd.Next()
		if msg == nil {
			break
		}
		buffer, err := msg.Collect()
		if err != nil {
			return SyncResult{}, fmt.Errorf("fetch message: %w", err)
		}
		summary := MessageSummary{UID: uint32(buffer.UID), Date: buffer.InternalDate}
		if buffer.Envelope != nil {
			summary.Subject = strings.TrimSpace(buffer.Envelope.Subject)
			summary.From = formatAddresses(buffer.Envelope.From)
			if !buffer.Envelope.Date.IsZero() {
				summary.Date = buffer.Envelope.Date
			}
		}
		if summary.Subject == "" {
			summary.Subject = "(no subject)"
		}
		if summary.UID > nextState.LastUID {
			nextState.LastUID = summary.UID
		}
		summaries = append(summaries, summary)
	}
	if err := cmd.Close(); err != nil {
		return SyncResult{}, fmt.Errorf("fetch close: %w", err)
	}
	return SyncResult{Messages: summaries, State: nextState}, nil
}

func (c *Client) connectAuthenticated(ctx context.Context) (*imapclient.Client, error) {
	client, err := c.connect(ctx)
	if err != nil {
		return nil, err
	}
	if err := c.authenticate(client); err != nil {
		client.Close()
		return nil, err
	}
	return client, nil
}

func (c *Client) connect(ctx context.Context) (*imapclient.Client, error) {
	if c.Host == "" || c.Port <= 0 {
		return nil, fmt.Errorf("imap host and port are required")
	}
	timeout := c.Timeout
	if timeout <= 0 {
		timeout = defaultTimeout
	}
	tlsConfig := c.TLSConfig
	if tlsConfig == nil {
		tlsConfig = &tls.Config{
			ServerName: strings.ToLower(c.Host),
			MinVersion: tls.VersionTLS12,
		}
	} else {
		tlsConfig = tlsConfig.Clone()
		if tlsConfig.ServerName == "" {
			tlsConfig.ServerName = strings.ToLower(c.Host)
		}
		if tlsConfig.MinVersion == 0 {
			tlsConfig.MinVersion = tls.VersionTLS12
		}
	}
	options := &imapclient.Options{
		Dialer:    &net.Dialer{Timeout: timeout},
		TLSConfig: tlsConfig,
	}

	address := net.JoinHostPort(c.Host, fmt.Sprintf("%d", c.Port))
	var client *imapclient.Client
	var err error
	switch strings.ToLower(strings.TrimSpace(c.Security)) {
	case SecurityTLS:
		client, err = imapclient.DialTLS(address, options)
	case SecurityStartTLS:
		client, err = imapclient.DialStartTLS(address, options)
	default:
		return nil, fmt.Errorf("imap security must be tls or starttls")
	}
	if ctxErr := ctx.Err(); ctxErr != nil {
		if client != nil {
			client.Close()
		}
		return nil, ctxErr
	}
	if err != nil {
		return nil, fmt.Errorf("imap connect %s: %w", address, err)
	}
	return client, nil
}

func (c *Client) authenticate(client *imapclient.Client) error {
	if strings.TrimSpace(c.Username) == "" || strings.TrimSpace(c.Secret) == "" {
		return fmt.Errorf("imap username and credential are required")
	}

	switch strings.ToLower(strings.TrimSpace(c.AuthMode)) {
	case AuthModeOAuth:
		caps := client.Caps()
		if caps.Has(goimap.AuthCap(sasl.OAuthBearer)) {
			return client.Authenticate(sasl.NewOAuthBearerClient(&sasl.OAuthBearerOptions{
				Username: c.Username,
				Token:    c.Secret,
				Host:     c.Host,
				Port:     c.Port,
			}))
		}
		if caps.Has(goimap.AuthCap("XOAUTH2")) {
			return client.Authenticate(xoauth2Client{username: c.Username, token: c.Secret})
		}
		return fmt.Errorf("imap server does not advertise OAuth SASL")
	case "", AuthModePassword:
		caps := client.Caps()
		if caps.Has(goimap.AuthCap(sasl.Plain)) {
			return client.Authenticate(sasl.NewPlainClient("", c.Username, c.Secret))
		}
		if caps.Has(goimap.CapLoginDisabled) {
			return fmt.Errorf("imap server disabled password LOGIN and does not advertise PLAIN")
		}
		return client.Login(c.Username, c.Secret).Wait()
	default:
		return fmt.Errorf("imap auth mode is not supported")
	}
}

type xoauth2Client struct {
	username string
	token    string
}

func (c xoauth2Client) Start() (string, []byte, error) {
	response := fmt.Sprintf("user=%s\x01auth=Bearer %s\x01\x01", c.username, c.token)
	return "XOAUTH2", []byte(response), nil
}

func (c xoauth2Client) Next(challenge []byte) ([]byte, error) {
	return nil, nil
}

func formatAddresses(addresses []goimap.Address) string {
	if len(addresses) == 0 {
		return ""
	}
	parts := make([]string, 0, len(addresses))
	for _, addr := range addresses {
		email := addr.Addr()
		if email == "" {
			continue
		}
		if addr.Name == "" {
			parts = append(parts, email)
			continue
		}
		parts = append(parts, fmt.Sprintf("%s <%s>", addr.Name, email))
	}
	return strings.Join(parts, ", ")
}
