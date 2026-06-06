package mail

import (
	"context"
	"crypto/sha256"
	"crypto/tls"
	"database/sql"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"time"

	"komorebi/internal/credentials"
	"komorebi/internal/db"
	"komorebi/internal/imap"

	gomail "github.com/wneessen/go-mail"
)

type credentialStore interface {
	Save(accountID string, data credentials.CredentialData) error
	Get(accountID string) (credentials.CredentialData, error)
	Delete(accountID string) error
}

type winCredentialStore struct{}

func (winCredentialStore) Save(accountID string, data credentials.CredentialData) error {
	return credentials.Save(accountID, data)
}

func (winCredentialStore) Get(accountID string) (credentials.CredentialData, error) {
	return credentials.Get(accountID)
}

func (winCredentialStore) Delete(accountID string) error {
	return credentials.Delete(accountID)
}

type oauthExchangeFunc func(context.Context, providerPolicy, oauthPending, OAuthCompleteRequest) (oauthTokenResponse, error)

type syncJob struct {
	cancel          context.CancelFunc
	intervalSeconds int
	folderPolicy    string
	lastRun         int64
	lastError       string
	lastFolders     []string
}

// Service exposes the local mail cache and provider-safe operations to Wails.
type Service struct {
	db                 *sql.DB
	credentials        credentialStore
	exchangeOAuthToken oauthExchangeFunc
	httpClient         *http.Client
	appDataDir         string
	downloadsDir       string
	imapTLSConfig      *tls.Config
	smtpTLSConfig      *tls.Config
	now                func() time.Time
	oauthCallbacksMu   sync.Mutex
	oauthCallbacks     map[string]*oauthCallbackSession
	syncJobsMu         sync.Mutex
	syncJobs           map[string]*syncJob
	syncRestored       bool
}

// NewService creates the Wails service backed by SQLite and Windows Credential Manager.
func NewService(database *sql.DB) *Service {
	s := &Service{
		db:             database,
		credentials:    winCredentialStore{},
		now:            time.Now,
		appDataDir:     defaultAppDataDir(),
		downloadsDir:   defaultDownloadsDir(),
		oauthCallbacks: map[string]*oauthCallbackSession{},
		syncJobs:       map[string]*syncJob{},
	}
	s.exchangeOAuthToken = s.exchangeOAuthTokenHTTP
	return s
}

func defaultAppDataDir() string {
	appData, err := os.UserConfigDir()
	if err != nil || strings.TrimSpace(appData) == "" {
		return filepath.Join(".", "Komorebi")
	}
	return filepath.Join(appData, "Komorebi")
}

func defaultDownloadsDir() string {
	home, err := os.UserHomeDir()
	if err != nil || strings.TrimSpace(home) == "" {
		return filepath.Join(".", "Komorebi Attachments")
	}
	return filepath.Join(home, "Downloads", "Komorebi Attachments")
}

// LoadMailShell returns real data from SQLite for the accountless shell.
func (s *Service) LoadMailShell() (MailShellData, error) {
	if err := db.EnsureSeedAccount(s.db); err != nil {
		return MailShellData{}, err
	}
	s.restoreBackgroundSyncJobs()

	data := MailShellData{}
	accounts, err := s.ListAccounts()
	if err != nil {
		return data, err
	}
	for _, account := range accounts {
		data.Accounts = append(data.Accounts, AccountView{
			ID:           account.ID,
			Email:        account.Email,
			DisplayName:  account.DisplayName,
			Provider:     account.ProviderKind,
			AuthMode:     account.AuthMode,
			IMAPHost:     account.IMAPHost,
			IMAPPort:     account.IMAPPort,
			IMAPSecurity: account.IMAPSecurity,
			SMTPHost:     account.SMTPHost,
			SMTPPort:     account.SMTPPort,
			SMTPSecurity: account.SMTPSecurity,
			Safety:       safetyForAccount(account.ProviderKind, account.IMAPSecurity, account.SMTPSecurity),
		})
	}

	data.Folders, err = s.listFolders()
	if err != nil {
		return data, err
	}

	inboxID, err := s.firstInboxFolderID()
	if err != nil {
		return data, err
	}
	if inboxID == "" {
		return data, nil
	}

	page, err := s.LoadMessagePage(MessagePageRequest{FolderID: inboxID, Limit: defaultMessageLimit})
	if err != nil {
		return data, err
	}
	data.Messages = page.Messages
	data.HasMore = page.HasMore
	if err := s.db.QueryRow(`SELECT COUNT(*) FROM messages WHERE folder_id = ?`, inboxID).Scan(&data.Total); err != nil {
		return data, err
	}

	return data, nil
}

// LoadMessagePage returns the next page of messages for a validated folder.
func (s *Service) LoadMessagePage(req MessagePageRequest) (MessagePageResponse, error) {
	folderID, err := validateID("folder id", req.FolderID)
	if err != nil {
		return MessagePageResponse{}, err
	}
	cursor, err := validateOffset(req.Cursor)
	if err != nil {
		return MessagePageResponse{}, err
	}
	limit := validateLimit(req.Limit, defaultMessageLimit, maxMessageLimit)
	if err := s.requireFolder(folderID); err != nil {
		return MessagePageResponse{}, err
	}

	rows, err := s.db.Query(`
		SELECT id, folder_id, COALESCE(subject, '(no subject)'),
		       COALESCE(sender_name, ''), COALESCE(sender_email, ''),
		       COALESCE(date_received, 0), is_read, is_flagged, COALESCE(snippet, '')
		FROM messages
		WHERE folder_id = ?
		ORDER BY date_received DESC
		LIMIT ? OFFSET ?
	`, folderID, limit, cursor)
	if err != nil {
		return MessagePageResponse{}, err
	}
	defer rows.Close()

	messages, err := scanMessageRows(rows)
	if err != nil {
		return MessagePageResponse{}, err
	}

	var total int
	if err := s.db.QueryRow(`SELECT COUNT(*) FROM messages WHERE folder_id = ?`, folderID).Scan(&total); err != nil {
		return MessagePageResponse{}, err
	}
	nextCursor := cursor + int64(len(messages))
	return MessagePageResponse{
		Messages:   messages,
		NextCursor: nextCursor,
		HasMore:    int(nextCursor) < total,
	}, nil
}

// LoadMessageDetail returns a sanitized message body for safe WebView rendering.
func (s *Service) LoadMessageDetail(req MessageDetailRequest) (MessageDetail, error) {
	messageID, err := validateID("message id", req.MessageID)
	if err != nil {
		return MessageDetail{}, err
	}

	var detail MessageDetail
	var bodyText, bodyHTML string
	err = s.db.QueryRow(`
		SELECT id, account_id, folder_id, COALESCE(subject, '(no subject)'),
		       COALESCE(sender_name, ''), COALESCE(sender_email, ''),
		       COALESCE(date_received, 0), COALESCE(body_text, ''), COALESCE(body_html, '')
		FROM messages
		WHERE id = ?
	`, messageID).Scan(
		&detail.ID,
		&detail.AccountID,
		&detail.FolderID,
		&detail.Subject,
		&detail.SenderName,
		&detail.SenderEmail,
		&detail.DateReceived,
		&bodyText,
		&bodyHTML,
	)
	if err == sql.ErrNoRows {
		return MessageDetail{}, fmt.Errorf("message not found")
	}
	if err != nil {
		return MessageDetail{}, err
	}

	if strings.TrimSpace(bodyHTML) != "" {
		remoteAllowed := s.remoteContentAllowed(detail.AccountID, detail.ID, detail.SenderEmail)
		safe := sanitizeMessageHTMLWithOptions(bodyHTML, sanitizeOptions{AllowRemoteContent: remoteAllowed})
		detail.SafeHTML = safe.HTML
		detail.BodyText = safe.Text
		detail.RemoteContentBlocked = safe.RemoteContentBlocked
		detail.RemoteContentAllowed = remoteAllowed
		detail.UnsafeHTMLRemoved = safe.UnsafeRemoved
		detail.RenderMode = "sanitized-html"
		if detail.BodyText == "" {
			detail.BodyText = bodyText
		}
		return detail, nil
	}

	detail.BodyText = strings.TrimSpace(bodyText)
	detail.SafeHTML = textToSafeHTML(detail.BodyText)
	detail.RenderMode = "plain-text"
	return detail, nil
}

func (s *Service) SetRemoteContentPreference(req RemoteContentPreferenceRequest) error {
	messageID, err := validateID("message id", req.MessageID)
	if err != nil {
		return err
	}
	scope := strings.ToLower(strings.TrimSpace(req.Scope))
	if scope == "" {
		scope = "message"
	}
	if scope != "message" && scope != "sender" {
		return fmt.Errorf("remote content scope is invalid")
	}
	var accountID, senderEmail string
	err = s.db.QueryRow(`
		SELECT account_id, COALESCE(sender_email, '')
		FROM messages
		WHERE id = ?
	`, messageID).Scan(&accountID, &senderEmail)
	if err == sql.ErrNoRows {
		return fmt.Errorf("message not found")
	}
	if err != nil {
		return err
	}
	targetID := messageID
	if scope == "sender" {
		senderEmail, err = validateEmail(senderEmail)
		if err != nil {
			return fmt.Errorf("message sender is not a valid email address")
		}
		targetID = strings.ToLower(senderEmail)
	}
	if req.Allowed {
		_, err = s.db.Exec(`
			INSERT OR IGNORE INTO remote_content_preferences (id, account_id, scope, target_id, created_at)
			VALUES (?, ?, ?, ?, unixepoch())
		`, "remote-"+accountID+"-"+scope+"-"+targetID, accountID, scope, targetID)
		return err
	}
	_, err = s.db.Exec(`
		DELETE FROM remote_content_preferences
		WHERE account_id = ? AND scope = ? AND target_id = ?
	`, accountID, scope, targetID)
	return err
}

func (s *Service) OpenExternalLink(req OpenExternalLinkRequest) error {
	link := strings.TrimSpace(req.URL)
	if link == "" || strings.ContainsAny(link, "\r\n\t") {
		return fmt.Errorf("link is invalid")
	}
	parsed, err := url.Parse(link)
	if err != nil || parsed.Scheme == "" {
		return fmt.Errorf("link is invalid")
	}
	switch strings.ToLower(parsed.Scheme) {
	case "https", "http", "mailto":
	default:
		return fmt.Errorf("link scheme is not allowed")
	}
	if parsed.Scheme != "mailto" && parsed.Host == "" {
		return fmt.Errorf("link host is invalid")
	}
	if runtime.GOOS != "windows" {
		return fmt.Errorf("external link opening is only implemented on Windows")
	}
	return exec.Command("rundll32", "url.dll,FileProtocolHandler", parsed.String()).Start()
}

// MarkRead performs a validated UPDATE on the SQLite cache.
func (s *Service) MarkRead(req MarkReadRequest) error {
	messageID, err := validateID("message id", req.MessageID)
	if err != nil {
		return err
	}
	result, err := s.db.Exec(`UPDATE messages SET is_read = ? WHERE id = ?`, boolToInt(req.Read), messageID)
	if err != nil {
		return err
	}
	if changed, _ := result.RowsAffected(); changed == 0 {
		return fmt.Errorf("message not found")
	}
	return nil
}

// SearchMail performs an FTS5 search after converting user input into safe terms.
func (s *Service) SearchMail(req SearchRequest) ([]MessageRow, error) {
	query, err := normalizeSearchQuery(req.Query)
	if err != nil {
		return nil, err
	}
	if query == "" {
		return []MessageRow{}, nil
	}
	limit := validateLimit(req.Limit, 50, maxMessageLimit)

	rows, err := s.db.Query(`
		SELECT m.id, m.folder_id, COALESCE(m.subject, '(no subject)'),
		       COALESCE(m.sender_name, ''), COALESCE(m.sender_email, ''),
		       COALESCE(m.date_received, 0), m.is_read, m.is_flagged,
		       COALESCE(m.snippet, '')
		FROM messages_fts f
		JOIN messages m ON m.rowid = f.rowid
		WHERE messages_fts MATCH ?
		ORDER BY rank
		LIMIT ?
	`, query, limit)
	if err != nil {
		return nil, fmt.Errorf("fts search: %w", err)
	}
	defer rows.Close()

	return scanMessageRows(rows)
}

// ListDrafts returns local drafts from SQLite.
func (s *Service) ListDrafts() ([]Draft, error) {
	rows, err := s.db.Query(`
		SELECT id, account_id, COALESCE(subject,''), COALESCE(to_addresses,'[]'),
		       COALESCE(body_html,''), COALESCE(updated_at, 0)
		FROM composer_drafts
		ORDER BY updated_at DESC
		LIMIT 50
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var drafts []Draft
	for rows.Next() {
		var d Draft
		if err := rows.Scan(&d.ID, &d.AccountID, &d.Subject, &d.To, &d.BodyHTML, &d.UpdatedAt); err != nil {
			return nil, err
		}
		drafts = append(drafts, d)
	}
	return drafts, rows.Err()
}

// SaveDraft persists a sanitized draft row.
func (s *Service) SaveDraft(d Draft) error {
	id, err := validateID("draft id", d.ID)
	if err != nil {
		return err
	}
	accountID, err := validateID("account id", d.AccountID)
	if err != nil {
		return err
	}
	if err := s.requireAccount(accountID); err != nil {
		return err
	}
	subject, err := validateSubject(d.Subject)
	if err != nil {
		return err
	}
	toJSON, _, err := normalizeAddressJSON(d.To, true)
	if err != nil {
		return err
	}
	bodyHTML, err := validateHTMLBody(d.BodyHTML)
	if err != nil {
		return err
	}
	safe := sanitizeMessageHTML(bodyHTML)

	_, err = s.db.Exec(`
		INSERT OR REPLACE INTO composer_drafts
			(id, account_id, subject, to_addresses, body_html, body_text, updated_at)
		VALUES (?, ?, ?, ?, ?, ?, unixepoch())
	`, id, accountID, subject, toJSON, safe.HTML, safe.Text)
	return err
}

// DeleteDraft removes a draft.
func (s *Service) DeleteDraft(id string) error {
	draftID, err := validateID("draft id", id)
	if err != nil {
		return err
	}
	_, err = s.db.Exec(`DELETE FROM composer_drafts WHERE id = ?`, draftID)
	return err
}

// SendDraft sends a local demo draft or delegates to provider-safe SMTP for real accounts.
func (s *Service) SendDraft(draftID string) error {
	draftID, err := validateID("draft id", draftID)
	if err != nil {
		return err
	}

	var d Draft
	if err := s.db.QueryRow(`
		SELECT id, account_id, COALESCE(subject,''), COALESCE(to_addresses,'[]'), COALESCE(body_html,''), COALESCE(updated_at, 0)
		FROM composer_drafts
		WHERE id = ?
	`, draftID).Scan(&d.ID, &d.AccountID, &d.Subject, &d.To, &d.BodyHTML, &d.UpdatedAt); err != nil {
		return err
	}

	account, err := s.getAccount(d.AccountID)
	if err != nil {
		return err
	}
	toJSON, recipients, err := normalizeAddressJSON(d.To, false)
	if err != nil {
		return err
	}
	d.To = toJSON

	if account.ProviderKind == providerLocal {
		return s.createLocalSentMessage(d, account.Email)
	}

	if err := s.SendEmail(SendEmailRequest{
		AccountID: d.AccountID,
		From:      account.Email,
		To:        recipients,
		Subject:   d.Subject,
		BodyHTML:  d.BodyHTML,
	}); err != nil {
		return err
	}
	_, _ = s.db.Exec(`DELETE FROM composer_drafts WHERE id = ?`, draftID)
	return nil
}

// SendEmail performs SMTP send using validated account-specific settings.
func (s *Service) SendEmail(req SendEmailRequest) error {
	accountID, err := validateID("account id", req.AccountID)
	if err != nil {
		return err
	}
	account, err := s.getAccount(accountID)
	if err != nil {
		return err
	}
	if account.ProviderKind == providerLocal {
		return fmt.Errorf("local demo accounts cannot send provider mail")
	}

	from, err := validateEmail(req.From)
	if err != nil {
		return err
	}
	if !strings.EqualFold(from, account.Email) {
		return fmt.Errorf("from address must match the selected account")
	}
	recipients, err := validateAddressList(req.To, false)
	if err != nil {
		return err
	}
	subject, err := validateSubject(req.Subject)
	if err != nil {
		return err
	}
	bodyHTML, err := validateHTMLBody(req.BodyHTML)
	if err != nil {
		return err
	}
	safe := sanitizeMessageHTML(bodyHTML)

	client, err := smtpClient(account, s.smtpTLSConfig)
	if err != nil {
		return err
	}
	secret, authMode, err := s.smtpSecret(account)
	if err != nil {
		return err
	}
	if authMode == authModeOAuth {
		client.SetSMTPAuth(gomail.SMTPAuthXOAUTH2)
	} else {
		client.SetSMTPAuth(gomail.SMTPAuthPlain)
	}
	client.SetUsername(account.Email)
	client.SetPassword(secret)

	msg := gomail.NewMsg()
	if err := msg.From(from); err != nil {
		return err
	}
	if err := msg.To(recipients...); err != nil {
		return err
	}
	msg.Subject(subject)
	msg.SetBodyString(gomail.TypeTextHTML, safe.HTML)

	if err := client.DialAndSendWithContext(context.Background(), msg); err != nil {
		return fmt.Errorf("SMTP send failed: %w", err)
	}

	return s.cacheSentMessage(account, subject, safe.HTML, safe.Text)
}

// SyncAccount performs provider-safe IMAP sync over encrypted account-specific settings.
func (s *Service) SyncAccount(req SyncAccountRequest) error {
	accountID, err := validateID("account id", req.AccountID)
	if err != nil {
		return err
	}
	_, err = s.syncAccountContext(context.Background(), accountID, validateFolderPolicy(req.FolderPolicy))
	return err
}

func (s *Service) StartBackgroundSync(req BackgroundSyncRequest) (BackgroundSyncStatus, error) {
	accountID, err := validateID("account id", req.AccountID)
	if err != nil {
		return BackgroundSyncStatus{}, err
	}
	account, err := s.getAccount(accountID)
	if err != nil {
		return BackgroundSyncStatus{}, err
	}
	if account.ProviderKind == providerLocal {
		return BackgroundSyncStatus{}, fmt.Errorf("local demo accounts do not have a remote sync endpoint")
	}
	intervalSeconds := req.IntervalSeconds
	if intervalSeconds < 60 {
		intervalSeconds = 300
	}
	if intervalSeconds > 24*60*60 {
		intervalSeconds = 24 * 60 * 60
	}
	folderPolicy := validateFolderPolicy(req.FolderPolicy)

	s.StopBackgroundSync(SyncAccountRequest{AccountID: accountID})
	ctx, cancel := context.WithCancel(context.Background())
	job := &syncJob{cancel: cancel, intervalSeconds: intervalSeconds, folderPolicy: folderPolicy}
	s.syncJobsMu.Lock()
	s.syncJobs[accountID] = job
	s.syncJobsMu.Unlock()
	if err := s.saveSyncPreference(accountID, true, intervalSeconds, folderPolicy); err != nil {
		_ = s.StopBackgroundSync(SyncAccountRequest{AccountID: accountID})
		return BackgroundSyncStatus{}, err
	}

	go s.runSyncJob(ctx, accountID, job)
	return s.backgroundStatusFor(accountID, job, true), nil
}

func (s *Service) StopBackgroundSync(req SyncAccountRequest) error {
	accountID, err := validateID("account id", req.AccountID)
	if err != nil {
		return err
	}
	s.syncJobsMu.Lock()
	job, ok := s.syncJobs[accountID]
	if ok {
		delete(s.syncJobs, accountID)
	}
	s.syncJobsMu.Unlock()
	if ok {
		job.cancel()
	}
	_ = s.saveSyncPreference(accountID, false, 300, defaultFolderPolicy)
	return nil
}

func (s *Service) GetBackgroundSyncStatus() ([]BackgroundSyncStatus, error) {
	s.syncJobsMu.Lock()
	defer s.syncJobsMu.Unlock()
	statuses := make([]BackgroundSyncStatus, 0, len(s.syncJobs))
	for accountID, job := range s.syncJobs {
		statuses = append(statuses, s.backgroundStatusFor(accountID, job, true))
	}
	return statuses, nil
}

func (s *Service) runSyncJob(ctx context.Context, accountID string, job *syncJob) {
	folders, err := s.syncAccountContext(ctx, accountID, job.folderPolicy)
	s.recordSyncJobRun(accountID, folders, err)
	ticker := time.NewTicker(time.Duration(job.intervalSeconds) * time.Second)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			folders, err := s.syncAccountContext(ctx, accountID, job.folderPolicy)
			s.recordSyncJobRun(accountID, folders, err)
		}
	}
}

func (s *Service) recordSyncJobRun(accountID string, folders []string, runErr error) {
	s.syncJobsMu.Lock()
	defer s.syncJobsMu.Unlock()
	job, ok := s.syncJobs[accountID]
	if !ok {
		return
	}
	job.lastRun = s.now().Unix()
	job.lastFolders = append([]string(nil), folders...)
	if runErr != nil && !errors.Is(runErr, context.Canceled) {
		job.lastError = runErr.Error()
	} else {
		job.lastError = ""
	}
}

func (s *Service) backgroundStatusFor(accountID string, job *syncJob, running bool) BackgroundSyncStatus {
	return BackgroundSyncStatus{
		AccountID:       accountID,
		Running:         running,
		LastRun:         job.lastRun,
		LastError:       job.lastError,
		IntervalSeconds: job.intervalSeconds,
		FolderPolicy:    job.folderPolicy,
		LastFolders:     append([]string(nil), job.lastFolders...),
	}
}

const defaultFolderPolicy = "special"

type syncFolderCandidate struct {
	localFolderID string
	specialUse    string
	remoteName    string
}

func validateFolderPolicy(value string) string {
	switch strings.ToLower(strings.TrimSpace(value)) {
	case "inbox":
		return "inbox"
	case "", defaultFolderPolicy:
		return defaultFolderPolicy
	default:
		return defaultFolderPolicy
	}
}

func (s *Service) restoreBackgroundSyncJobs() {
	s.syncJobsMu.Lock()
	if s.syncRestored {
		s.syncJobsMu.Unlock()
		return
	}
	s.syncRestored = true
	s.syncJobsMu.Unlock()

	rows, err := s.db.Query(`
		SELECT account_id, interval_seconds, folder_policy
		FROM sync_preferences
		WHERE enabled = 1
	`)
	if err != nil {
		return
	}
	defer rows.Close()
	for rows.Next() {
		var req BackgroundSyncRequest
		if err := rows.Scan(&req.AccountID, &req.IntervalSeconds, &req.FolderPolicy); err != nil {
			continue
		}
		_, _ = s.StartBackgroundSync(req)
	}
}

func (s *Service) saveSyncPreference(accountID string, enabled bool, intervalSeconds int, folderPolicy string) error {
	accountID, err := validateID("account id", accountID)
	if err != nil {
		return err
	}
	if intervalSeconds < 60 {
		intervalSeconds = 300
	}
	if intervalSeconds > 24*60*60 {
		intervalSeconds = 24 * 60 * 60
	}
	folderPolicy = validateFolderPolicy(folderPolicy)
	_, err = s.db.Exec(`
		INSERT INTO sync_preferences (account_id, enabled, interval_seconds, folder_policy, updated_at)
		VALUES (?, ?, ?, ?, unixepoch())
		ON CONFLICT(account_id) DO UPDATE SET
			enabled = excluded.enabled,
			interval_seconds = excluded.interval_seconds,
			folder_policy = excluded.folder_policy,
			updated_at = unixepoch()
	`, accountID, boolToInt(enabled), intervalSeconds, folderPolicy)
	return err
}

func (s *Service) syncFolderCandidates(ctx context.Context, accountID string, client *imap.Client, folderPolicy string) ([]syncFolderCandidate, error) {
	folders, err := s.localSyncFolders(accountID, folderPolicy)
	if err != nil {
		return nil, err
	}
	if len(folders) == 0 {
		return []syncFolderCandidate{}, nil
	}
	remoteMailboxes, err := client.ListMailboxes(ctx)
	if err != nil {
		for i := range folders {
			if folders[i].specialUse == "inbox" {
				folders[i].remoteName = "INBOX"
				return []syncFolderCandidate{folders[i]}, nil
			}
		}
		return nil, err
	}
	candidates := make([]syncFolderCandidate, 0, len(folders))
	for _, folder := range folders {
		remoteName := remoteMailboxForSpecialUse(folder.specialUse, remoteMailboxes)
		if remoteName == "" {
			continue
		}
		folder.remoteName = remoteName
		candidates = append(candidates, folder)
	}
	return candidates, nil
}

func (s *Service) localSyncFolders(accountID, folderPolicy string) ([]syncFolderCandidate, error) {
	accountID, err := validateID("account id", accountID)
	if err != nil {
		return nil, err
	}
	allowed := map[string]bool{"inbox": true}
	if validateFolderPolicy(folderPolicy) == defaultFolderPolicy {
		allowed = map[string]bool{"inbox": true, "sent": true, "drafts": true, "archive": true, "trash": true}
	}
	rows, err := s.db.Query(`
		SELECT id, COALESCE(special_use, '')
		FROM folders
		WHERE account_id = ?
		ORDER BY CASE special_use
			WHEN 'inbox' THEN 1
			WHEN 'sent' THEN 2
			WHEN 'drafts' THEN 3
			WHEN 'archive' THEN 4
			WHEN 'trash' THEN 5
			ELSE 10
		END
	`, accountID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var result []syncFolderCandidate
	for rows.Next() {
		var item syncFolderCandidate
		if err := rows.Scan(&item.localFolderID, &item.specialUse); err != nil {
			return nil, err
		}
		if allowed[item.specialUse] {
			result = append(result, item)
		}
	}
	return result, rows.Err()
}

func remoteMailboxForSpecialUse(specialUse string, remote []imap.MailboxInfo) string {
	if specialUse == "inbox" {
		for _, mailbox := range remote {
			if strings.EqualFold(mailbox.Name, "INBOX") {
				return mailbox.Name
			}
		}
		return "INBOX"
	}
	for _, mailbox := range remote {
		if mailboxHasSpecialUse(mailbox, specialUse) {
			return mailbox.Name
		}
	}
	names := fallbackRemoteFolderNames(specialUse)
	for _, expected := range names {
		for _, mailbox := range remote {
			if strings.EqualFold(mailbox.Name, expected) {
				return mailbox.Name
			}
		}
	}
	return ""
}

func mailboxHasSpecialUse(mailbox imap.MailboxInfo, specialUse string) bool {
	want := "\\" + strings.ToLower(specialUse)
	if specialUse == "trash" {
		want = "\\trash"
	}
	for _, attr := range mailbox.Attrs {
		if strings.ToLower(attr) == want {
			return true
		}
	}
	return false
}

func fallbackRemoteFolderNames(specialUse string) []string {
	switch specialUse {
	case "sent":
		return []string{"Sent", "Sent Items"}
	case "drafts":
		return []string{"Drafts"}
	case "archive":
		return []string{"Archive", "Archives"}
	case "trash":
		return []string{"Trash", "Deleted Items", "Deleted Messages"}
	default:
		return nil
	}
}

func (s *Service) syncAccountContext(ctx context.Context, accountID, folderPolicy string) ([]string, error) {
	account, err := s.getAccount(accountID)
	if err != nil {
		return nil, err
	}
	if account.ProviderKind == providerLocal {
		return nil, fmt.Errorf("local demo accounts do not have a remote sync endpoint")
	}
	if _, err := validateSecurity(account.IMAPSecurity); err != nil {
		return nil, err
	}

	secret, authMode, err := s.imapSecret(account)
	if err != nil {
		return nil, err
	}
	var client *imap.Client
	if authMode == authModeOAuth {
		client = imap.NewOAuthClient(account.IMAPHost, account.IMAPPort, account.Email, secret, account.IMAPSecurity)
	} else {
		client = imap.NewPasswordClient(account.IMAPHost, account.IMAPPort, account.Email, secret, account.IMAPSecurity)
	}
	client.TLSConfig = s.imapTLSConfig
	candidates, err := s.syncFolderCandidates(ctx, account.ID, client, folderPolicy)
	if err != nil {
		return nil, err
	}

	var synced []string
	var failures []string
	for _, candidate := range candidates {
		syncState, err := s.loadSyncState(account.ID, candidate.remoteName)
		if err != nil {
			return synced, err
		}
		result, err := client.FetchMailboxIncremental(ctx, candidate.remoteName, syncState)
		if err != nil {
			failures = append(failures, fmt.Sprintf("%s: %v", candidate.remoteName, err))
			continue
		}
		for _, msg := range result.Messages {
			messageID := fmt.Sprintf("sync-%s-%s-%d", account.ID, candidate.specialUse, msg.UID)
			dateReceived := s.now().Unix()
			if !msg.Date.IsZero() {
				dateReceived = msg.Date.Unix()
			}
			if _, err := s.db.Exec(`
				INSERT INTO messages
					(id, account_id, folder_id, subject, sender_email, date_received, snippet, body_text)
				VALUES (?, ?, ?, ?, ?, ?, ?, ?)
				ON CONFLICT(id) DO UPDATE SET
					subject = excluded.subject,
					sender_email = excluded.sender_email,
					date_received = excluded.date_received,
					snippet = excluded.snippet,
					body_text = excluded.body_text
			`, messageID, account.ID, candidate.localFolderID, msg.Subject, msg.From, dateReceived, msg.Subject, msg.Subject); err != nil {
				return synced, err
			}
		}
		if err := s.saveSyncState(account.ID, candidate.remoteName, result.State); err != nil {
			return synced, err
		}
		_, _ = s.db.Exec(`UPDATE folders SET last_sync_at = unixepoch() WHERE id = ?`, candidate.localFolderID)
		synced = append(synced, candidate.remoteName)
	}
	if len(synced) == 0 && len(failures) > 0 {
		return synced, fmt.Errorf("sync failed for all folders: %s", strings.Join(failures, "; "))
	}
	return synced, nil
}

// TestAndSyncIMAP validates and samples an encrypted IMAP endpoint without storing the password.
func (s *Service) TestAndSyncIMAP(req IMAPTestRequest) error {
	accountID, err := validateID("account id", req.AccountID)
	if err != nil {
		return err
	}
	host, err := validateHost(req.Host)
	if err != nil {
		return err
	}
	port, err := validatePort(req.Port)
	if err != nil {
		return err
	}
	security, err := validateSecurity(req.Security)
	if err != nil {
		return err
	}
	username, err := validateEmail(req.Username)
	if err != nil {
		return err
	}
	if strings.TrimSpace(req.Password) == "" {
		return fmt.Errorf("password is required for generic IMAP test")
	}

	client := imap.NewPasswordClient(host, port, username, req.Password, security)
	client.TLSConfig = s.imapTLSConfig
	if err := client.TestConnection(); err != nil {
		return fmt.Errorf("IMAP connection failed: %w", err)
	}
	samples, err := client.FetchInboxSample(20)
	if err != nil {
		return fmt.Errorf("fetch sample: %w", err)
	}
	folderID, err := s.folderIDForSpecial(accountID, "inbox")
	if err != nil {
		return err
	}
	for i, sm := range samples {
		_, _ = s.db.Exec(`
			INSERT OR IGNORE INTO messages
			(id, account_id, folder_id, subject, sender_email, date_received, is_read, snippet, body_text)
			VALUES (?, ?, ?, ?, ?, unixepoch() - ?, 0, ?, ?)
		`, fmt.Sprintf("imap-%s-%d", accountID, i), accountID, folderID, sm["subject"], sm["from"], i*3600, sm["subject"], sm["subject"])
	}
	return nil
}

// StageAttachment stores bytes under an app-generated path and returns non-path metadata only.
func (s *Service) StageAttachment(req StageAttachmentRequest) (AttachmentStageResult, error) {
	accountID, err := validateID("account id", req.AccountID)
	if err != nil {
		return AttachmentStageResult{}, err
	}
	messageID, err := validateID("message id", req.MessageID)
	if err != nil {
		return AttachmentStageResult{}, err
	}
	displayName, err := safeAttachmentFilename(req.DisplayName)
	if err != nil {
		return AttachmentStageResult{}, err
	}
	mime, err := validateMIME(req.Mime)
	if err != nil {
		return AttachmentStageResult{}, err
	}
	if len(req.Data) == 0 || len(req.Data) > maxAttachmentSize {
		return AttachmentStageResult{}, fmt.Errorf("attachment size is invalid")
	}

	baseDir := filepath.Join(s.attachmentRoot(), accountID, messageID)
	if err := os.MkdirAll(baseDir, 0700); err != nil {
		return AttachmentStageResult{}, err
	}

	sum := sha256.Sum256(req.Data)
	sha := hex.EncodeToString(sum[:])
	attachmentID := "att-" + sha[:16]
	storagePath := filepath.Join(baseDir, attachmentID+"-"+displayName)
	if err := os.WriteFile(storagePath, req.Data, 0600); err != nil {
		return AttachmentStageResult{}, err
	}

	_, err = s.db.Exec(`
		INSERT OR REPLACE INTO attachments
			(id, account_id, message_id, display_filename, mime_type, size_bytes, storage_path, sha256)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?)
	`, attachmentID, accountID, messageID, displayName, mime, len(req.Data), storagePath, sha)
	if err != nil {
		return AttachmentStageResult{}, err
	}
	return AttachmentStageResult{ID: attachmentID, DisplayName: displayName, SizeBytes: len(req.Data)}, nil
}

func (s *Service) ListAttachments(req MessageDetailRequest) ([]AttachmentView, error) {
	messageID, err := validateID("message id", req.MessageID)
	if err != nil {
		return nil, err
	}
	rows, err := s.db.Query(`
		SELECT id, message_id, COALESCE(display_filename, ''), COALESCE(mime_type, ''),
		       COALESCE(size_bytes, 0), COALESCE(created_at, 0)
		FROM attachments
		WHERE message_id = ?
		ORDER BY created_at, display_filename
	`, messageID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var attachments []AttachmentView
	for rows.Next() {
		var item AttachmentView
		if err := rows.Scan(&item.ID, &item.MessageID, &item.DisplayName, &item.Mime, &item.SizeBytes, &item.CreatedAt); err != nil {
			return nil, err
		}
		attachments = append(attachments, item)
	}
	return attachments, rows.Err()
}

func (s *Service) PreviewAttachment(req AttachmentRequest) (AttachmentPreview, error) {
	attachment, err := s.attachmentByID(req.AttachmentID)
	if err != nil {
		return AttachmentPreview{}, err
	}
	if attachment.sizeBytes > 2*1024*1024 {
		return AttachmentPreview{}, fmt.Errorf("attachment is too large to preview")
	}
	data, err := os.ReadFile(attachment.storagePath)
	if err != nil {
		return AttachmentPreview{}, err
	}
	kind := "unsupported"
	preview := AttachmentPreview{
		ID:          attachment.id,
		DisplayName: attachment.displayName,
		Mime:        attachment.mimeType,
		Kind:        kind,
		SizeBytes:   attachment.sizeBytes,
	}
	if strings.HasPrefix(attachment.mimeType, "text/") || attachment.mimeType == "application/json" {
		preview.Kind = "text"
		if len(data) > 128*1024 {
			data = data[:128*1024]
		}
		preview.Text = string(data)
		return preview, nil
	}
	if safePreviewImageMIME(attachment.mimeType) {
		preview.Kind = "image"
		preview.DataURL = "data:" + attachment.mimeType + ";base64," + base64.StdEncoding.EncodeToString(data)
		return preview, nil
	}
	return preview, nil
}

func (s *Service) SaveAttachment(req AttachmentRequest) (AttachmentSaveResult, error) {
	attachment, err := s.attachmentByID(req.AttachmentID)
	if err != nil {
		return AttachmentSaveResult{}, err
	}
	if err := os.MkdirAll(s.downloadsDir, 0700); err != nil {
		return AttachmentSaveResult{}, err
	}
	target := filepath.Join(s.downloadsDir, uniqueAttachmentFilename(s.downloadsDir, attachment.displayName))
	source, err := os.Open(attachment.storagePath)
	if err != nil {
		return AttachmentSaveResult{}, err
	}
	defer source.Close()
	destination, err := os.OpenFile(target, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		return AttachmentSaveResult{}, err
	}
	defer destination.Close()
	if _, err := io.Copy(destination, source); err != nil {
		return AttachmentSaveResult{}, err
	}
	return AttachmentSaveResult{ID: attachment.id, DisplayName: attachment.displayName, SavedPath: target}, nil
}

func (s *Service) DeleteAttachment(req AttachmentRequest) error {
	attachment, err := s.attachmentByID(req.AttachmentID)
	if err != nil {
		return err
	}
	_ = os.Remove(attachment.storagePath)
	_, err = s.db.Exec(`DELETE FROM attachments WHERE id = ?`, attachment.id)
	return err
}

// ListAccounts returns configured accounts without secrets.
func (s *Service) ListAccounts() ([]Account, error) {
	rows, err := s.db.Query(`
		SELECT id, email, COALESCE(display_name,''), provider_kind,
		       COALESCE(imap_host,''), COALESCE(imap_port,0), COALESCE(imap_security,'tls'),
		       COALESCE(smtp_host,''), COALESCE(smtp_port,0), COALESCE(smtp_security,'starttls')
		FROM accounts
		WHERE provider_kind != 'removed'
		ORDER BY CASE provider_kind WHEN 'local' THEN 0 ELSE 1 END, created_at
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var accounts []Account
	for rows.Next() {
		var account Account
		if err := rows.Scan(
			&account.ID,
			&account.Email,
			&account.DisplayName,
			&account.ProviderKind,
			&account.IMAPHost,
			&account.IMAPPort,
			&account.IMAPSecurity,
			&account.SMTPHost,
			&account.SMTPPort,
			&account.SMTPSecurity,
		); err != nil {
			return nil, err
		}
		account.AuthMode = authModeForProvider(account.ProviderKind)
		accounts = append(accounts, account)
	}
	return accounts, rows.Err()
}

func (s *Service) ValidateAccountSetup(req AccountSetupValidationRequest) (AccountSetupValidationResponse, error) {
	account, password, _, err := validateGenericAccountRequest(AddAccountRequest(req))
	if err != nil {
		return AccountSetupValidationResponse{}, err
	}
	return s.validateProposedAccount(account, password), nil
}

// AddAccount creates a generic TLS account and stores its secret only in WinCred.
func (s *Service) AddAccount(req AddAccountRequest) (string, error) {
	account, password, displayName, err := validateGenericAccountRequest(req)
	if err != nil {
		return "", err
	}
	validation := s.validateProposedAccount(account, password)
	if !validation.OK {
		return "", fmt.Errorf("account validation failed: %s", firstValidationFailure(validation.Checks))
	}

	accountID := accountIDForEmail(account.Email)
	tx, err := s.db.Begin()
	if err != nil {
		return "", err
	}
	defer tx.Rollback()

	_, err = tx.Exec(`
		INSERT INTO accounts (
			id, email, display_name, provider_kind, username, created_at, updated_at,
			imap_host, imap_port, imap_security,
			smtp_host, smtp_port, smtp_security
		) VALUES (?, ?, ?, ?, ?, unixepoch(), unixepoch(), ?, ?, ?, ?, ?, ?)
		ON CONFLICT(email) DO UPDATE SET
			display_name = excluded.display_name,
			provider_kind = excluded.provider_kind,
			username = excluded.username,
			updated_at = unixepoch(),
			imap_host = excluded.imap_host,
			imap_port = excluded.imap_port,
			imap_security = excluded.imap_security,
			smtp_host = excluded.smtp_host,
			smtp_port = excluded.smtp_port,
			smtp_security = excluded.smtp_security
	`, accountID, account.Email, displayName, providerGeneric, account.Email,
		account.IMAPHost, account.IMAPPort, account.IMAPSecurity,
		account.SMTPHost, account.SMTPPort, account.SMTPSecurity)
	if err != nil {
		return "", err
	}
	if err := ensureDefaultFoldersTx(tx, accountID); err != nil {
		return "", err
	}
	if err := s.credentials.Save(accountID, credentials.CredentialData{
		Secret:       password,
		AuthMode:     authModePassword,
		ProviderKind: providerGeneric,
		Username:     account.Email,
	}); err != nil {
		return "", fmt.Errorf("failed to store credential securely: %w", err)
	}
	if err := tx.Commit(); err != nil {
		_ = s.credentials.Delete(accountID)
		return "", err
	}
	return accountID, nil
}

func validateGenericAccountRequest(req AddAccountRequest) (accountRecord, string, string, error) {
	provider := normalizeProviderKind(req.ProviderKind)
	if provider == "" {
		return accountRecord{}, "", "", fmt.Errorf("provider is not supported")
	}
	if provider == providerLocal {
		return accountRecord{}, "", "", fmt.Errorf("local accounts are created only by the local cache seeder")
	}
	if provider == providerGmail || provider == providerMicrosoft {
		return accountRecord{}, "", "", fmt.Errorf("%s accounts require OAuth setup", provider)
	}

	email, err := validateEmail(req.Email)
	if err != nil {
		return accountRecord{}, "", "", err
	}
	displayName, err := validateDisplayName(req.DisplayName)
	if err != nil {
		return accountRecord{}, "", "", err
	}
	imapHost, err := validateHost(req.IMAPHost)
	if err != nil {
		return accountRecord{}, "", "", err
	}
	smtpHost, err := validateHost(req.SMTPHost)
	if err != nil {
		return accountRecord{}, "", "", err
	}
	imapPort, err := validatePort(req.IMAPPort)
	if err != nil {
		return accountRecord{}, "", "", err
	}
	smtpPort, err := validatePort(req.SMTPPort)
	if err != nil {
		return accountRecord{}, "", "", err
	}
	imapSecurity, err := validateSecurity(req.IMAPSecurity)
	if err != nil {
		return accountRecord{}, "", "", err
	}
	smtpSecurity, err := validateSecurity(req.SMTPSecurity)
	if err != nil {
		return accountRecord{}, "", "", err
	}
	password := strings.TrimSpace(req.Password)
	if password == "" {
		return accountRecord{}, "", "", fmt.Errorf("password is required for generic IMAP/SMTP")
	}

	return accountRecord{
		Email:        email,
		ProviderKind: providerGeneric,
		IMAPHost:     imapHost,
		IMAPPort:     imapPort,
		IMAPSecurity: imapSecurity,
		SMTPHost:     smtpHost,
		SMTPPort:     smtpPort,
		SMTPSecurity: smtpSecurity,
	}, password, displayName, nil
}

func (s *Service) validateProposedAccount(account accountRecord, password string) AccountSetupValidationResponse {
	checks := []ValidationCheck{
		{ID: "imap", Label: "IMAP", Status: "pass", Detail: "IMAP login succeeded."},
		{ID: "smtp", Label: "SMTP", Status: "pass", Detail: "SMTP authentication succeeded."},
	}
	client := imap.NewPasswordClient(account.IMAPHost, account.IMAPPort, account.Email, password, account.IMAPSecurity)
	client.TLSConfig = s.imapTLSConfig
	if err := client.TestConnection(); err != nil {
		checks[0].Status = "fail"
		checks[0].Detail = fmt.Sprintf("IMAP login failed: %v", err)
	}
	if err := s.testSMTPAuth(account, password, authModePassword); err != nil {
		checks[1].Status = "fail"
		checks[1].Detail = fmt.Sprintf("SMTP authentication failed: %v", err)
	}
	return AccountSetupValidationResponse{OK: checks[0].Status == "pass" && checks[1].Status == "pass", Checks: checks}
}

func (s *Service) testSMTPAuth(account accountRecord, secret, authMode string) error {
	client, err := smtpClient(account, s.smtpTLSConfig)
	if err != nil {
		return err
	}
	if authMode == authModeOAuth {
		client.SetSMTPAuth(gomail.SMTPAuthXOAUTH2)
	} else {
		client.SetSMTPAuth(gomail.SMTPAuthPlain)
	}
	client.SetUsername(account.Email)
	client.SetPassword(secret)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	if err := client.DialWithContext(ctx); err != nil {
		return err
	}
	return client.Close()
}

func firstValidationFailure(checks []ValidationCheck) string {
	for _, check := range checks {
		if check.Status != "pass" {
			return check.Detail
		}
	}
	return "unknown validation failure"
}

// RemoveAccount removes account metadata and credentials.
func (s *Service) RemoveAccount(req RemoveAccountRequest) error {
	accountID, err := validateID("account id", req.AccountID)
	if err != nil {
		return err
	}
	_ = s.credentials.Delete(accountID)
	if req.DeleteCache {
		_, err = s.db.Exec(`DELETE FROM accounts WHERE id = ?`, accountID)
		return err
	}
	_, err = s.db.Exec(`UPDATE accounts SET provider_kind = 'removed', updated_at = unixepoch() WHERE id = ?`, accountID)
	return err
}

func (s *Service) ListProviderPolicies() ([]ProviderPolicyView, error) {
	return providerPolicyViews(), nil
}

func (s *Service) GetDiagnosticsHealth() (DiagnosticsHealth, error) {
	health := DiagnosticsHealth{
		SchemaOK:       true,
		ProviderCounts: map[string]int{},
		GeneratedAt:    s.now().Unix(),
	}

	var err error
	if health.AccountCount, err = s.count(`SELECT COUNT(*) FROM accounts WHERE provider_kind != 'removed'`); err != nil {
		return health, err
	}
	if health.MessageCount, err = s.count(`SELECT COUNT(*) FROM messages`); err != nil {
		return health, err
	}
	if health.DraftCount, err = s.count(`SELECT COUNT(*) FROM composer_drafts`); err != nil {
		return health, err
	}
	if health.AttachmentCount, err = s.count(`SELECT COUNT(*) FROM attachments`); err != nil {
		return health, err
	}
	if health.PendingOAuth, err = s.count(`SELECT COUNT(*) FROM settings WHERE key LIKE 'oauth:pending:%'`); err != nil {
		return health, err
	}

	rows, err := s.db.Query(`SELECT provider_kind, COUNT(*) FROM accounts GROUP BY provider_kind`)
	if err != nil {
		return health, err
	}
	defer rows.Close()
	for rows.Next() {
		var provider string
		var count int
		if err := rows.Scan(&provider, &count); err != nil {
			return health, err
		}
		health.ProviderCounts[provider] = count
	}
	if err := rows.Err(); err != nil {
		return health, err
	}

	health.Checks = []DiagnosticCheck{
		{ID: "telemetry", Label: "Telemetry", Status: "pass", Detail: "No telemetry or ad endpoints are configured."},
		{ID: "credentials", Label: "Credential Storage", Status: "pass", Detail: "Account secrets are kept out of SQLite and frontend state."},
		{ID: "rendering", Label: "Message Rendering", Status: "pass", Detail: "HTML is sanitized before WebView rendering; remote images require explicit message or sender consent."},
		{ID: "ipc", Label: "IPC Validation", Status: "pass", Detail: "Provider, message, search, attachment, and account inputs are validated in Go."},
		{ID: "sync", Label: "Live Provider Sync", Status: "warn", Detail: "Encrypted IMAP supports password, OAUTHBEARER, and XOAUTH2 auth; full background sync policy is still limited."},
	}
	return health, nil
}

func (s *Service) ExportDiagnostics(req DiagnosticsExportRequest) (DiagnosticsExportResult, error) {
	health, err := s.GetDiagnosticsHealth()
	if err != nil {
		return DiagnosticsExportResult{}, err
	}
	accounts, err := s.redactedDiagnosticsAccounts()
	if err != nil {
		return DiagnosticsExportResult{}, err
	}
	bundle := map[string]any{
		"generatedAt": s.now().Unix(),
		"health":      health,
		"accounts":    accounts,
		"privacy": map[string]string{
			"credentials": "not exported",
			"paths":       "attachment storage paths and app-data paths are not exported",
			"mailData":    "message bodies and subjects are not exported",
		},
	}
	if req.IncludeSchema {
		versions, err := s.schemaVersions()
		if err != nil {
			return DiagnosticsExportResult{}, err
		}
		bundle["schemaVersions"] = versions
	}
	encoded, err := json.MarshalIndent(bundle, "", "  ")
	if err != nil {
		return DiagnosticsExportResult{}, err
	}
	dir := filepath.Join(s.appDataDir, "diagnostics")
	if err := os.MkdirAll(dir, 0700); err != nil {
		return DiagnosticsExportResult{}, err
	}
	generatedAt := s.now().Unix()
	path := filepath.Join(dir, fmt.Sprintf("komorebi-diagnostics-%d.json", generatedAt))
	if err := os.WriteFile(path, encoded, 0600); err != nil {
		return DiagnosticsExportResult{}, err
	}
	return DiagnosticsExportResult{Path: path, SizeBytes: len(encoded), GeneratedAt: generatedAt}, nil
}

func (s *Service) redactedDiagnosticsAccounts() ([]map[string]any, error) {
	rows, err := s.db.Query(`
		SELECT id, email, provider_kind, COALESCE(imap_host, ''), COALESCE(smtp_host, '')
		FROM accounts
		WHERE provider_kind != 'removed'
		ORDER BY created_at
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var result []map[string]any
	for rows.Next() {
		var id, email, providerKind, imapHost, smtpHost string
		if err := rows.Scan(&id, &email, &providerKind, &imapHost, &smtpHost); err != nil {
			return nil, err
		}
		result = append(result, map[string]any{
			"id":       id,
			"email":    redactEmail(email),
			"provider": providerKind,
			"imapHost": redactHost(imapHost),
			"smtpHost": redactHost(smtpHost),
		})
	}
	return result, rows.Err()
}

func (s *Service) schemaVersions() ([]int, error) {
	rows, err := s.db.Query(`SELECT version FROM schema_migrations ORDER BY version`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var versions []int
	for rows.Next() {
		var version int
		if err := rows.Scan(&version); err != nil {
			return nil, err
		}
		versions = append(versions, version)
	}
	return versions, rows.Err()
}

func redactEmail(value string) string {
	value = strings.TrimSpace(value)
	parts := strings.Split(value, "@")
	if len(parts) != 2 || parts[0] == "" || parts[1] == "" {
		return "redacted"
	}
	return parts[0][:1] + "***@" + redactHost(parts[1])
}

func redactHost(value string) string {
	value = strings.Trim(strings.ToLower(strings.TrimSpace(value)), ".")
	if value == "" {
		return ""
	}
	parts := strings.Split(value, ".")
	if len(parts) < 2 {
		return "***"
	}
	return "***." + strings.Join(parts[len(parts)-2:], ".")
}

func (s *Service) GetUIPreferences() (UIPreferences, error) {
	return UIPreferences{
		Language: s.settingOr("ui:language", "en"),
		Density:  s.settingOr("ui:density", "comfortable"),
	}, nil
}

func (s *Service) UpdateUIPreferences(req UIPreferencesRequest) (UIPreferences, error) {
	language := strings.TrimSpace(req.Language)
	switch language {
	case "", "en", "pt-PT":
	default:
		return UIPreferences{}, fmt.Errorf("language is not supported")
	}
	if language == "" {
		language = "en"
	}
	density := strings.TrimSpace(req.Density)
	switch density {
	case "", "comfortable", "compact":
	default:
		return UIPreferences{}, fmt.Errorf("density is not supported")
	}
	if density == "" {
		density = "comfortable"
	}
	if err := s.setSetting("ui:language", language); err != nil {
		return UIPreferences{}, err
	}
	if err := s.setSetting("ui:density", density); err != nil {
		return UIPreferences{}, err
	}
	return UIPreferences{Language: language, Density: density}, nil
}

type accountRecord struct {
	ID           string
	Email        string
	DisplayName  string
	ProviderKind string
	IMAPHost     string
	IMAPPort     int
	IMAPSecurity string
	SMTPHost     string
	SMTPPort     int
	SMTPSecurity string
}

type attachmentRecord struct {
	id          string
	displayName string
	mimeType    string
	sizeBytes   int
	storagePath string
}

func (s *Service) getAccount(accountID string) (accountRecord, error) {
	accountID, err := validateID("account id", accountID)
	if err != nil {
		return accountRecord{}, err
	}
	var account accountRecord
	err = s.db.QueryRow(`
		SELECT id, email, COALESCE(display_name,''), provider_kind,
		       COALESCE(imap_host,''), COALESCE(imap_port,0), COALESCE(imap_security,'tls'),
		       COALESCE(smtp_host,''), COALESCE(smtp_port,0), COALESCE(smtp_security,'starttls')
		FROM accounts
		WHERE id = ? AND provider_kind != 'removed'
	`, accountID).Scan(
		&account.ID,
		&account.Email,
		&account.DisplayName,
		&account.ProviderKind,
		&account.IMAPHost,
		&account.IMAPPort,
		&account.IMAPSecurity,
		&account.SMTPHost,
		&account.SMTPPort,
		&account.SMTPSecurity,
	)
	if err == sql.ErrNoRows {
		return accountRecord{}, fmt.Errorf("account not found")
	}
	if err != nil {
		return accountRecord{}, err
	}
	return account, nil
}

func (s *Service) attachmentRoot() string {
	base := strings.TrimSpace(s.appDataDir)
	if base == "" {
		base = defaultAppDataDir()
	}
	return filepath.Join(base, "cache", "attachments")
}

func (s *Service) attachmentByID(attachmentID string) (attachmentRecord, error) {
	attachmentID, err := validateID("attachment id", attachmentID)
	if err != nil {
		return attachmentRecord{}, err
	}
	var record attachmentRecord
	err = s.db.QueryRow(`
		SELECT id, COALESCE(display_filename, ''), COALESCE(mime_type, ''),
		       COALESCE(size_bytes, 0), COALESCE(storage_path, '')
		FROM attachments
		WHERE id = ?
	`, attachmentID).Scan(&record.id, &record.displayName, &record.mimeType, &record.sizeBytes, &record.storagePath)
	if err == sql.ErrNoRows {
		return attachmentRecord{}, fmt.Errorf("attachment not found")
	}
	if err != nil {
		return attachmentRecord{}, err
	}
	if record.displayName == "" {
		record.displayName = record.id
	}
	record.mimeType = strings.ToLower(strings.TrimSpace(record.mimeType))
	if record.mimeType == "" {
		record.mimeType = "application/octet-stream"
	}
	if err := ensurePathWithin(record.storagePath, s.attachmentRoot()); err != nil {
		return attachmentRecord{}, err
	}
	return record, nil
}

func ensurePathWithin(pathValue, rootValue string) error {
	pathValue = strings.TrimSpace(pathValue)
	rootValue = strings.TrimSpace(rootValue)
	if pathValue == "" || rootValue == "" {
		return fmt.Errorf("stored path is invalid")
	}
	absPath, err := filepath.Abs(pathValue)
	if err != nil {
		return err
	}
	absRoot, err := filepath.Abs(rootValue)
	if err != nil {
		return err
	}
	rel, err := filepath.Rel(absRoot, absPath)
	if err != nil {
		return err
	}
	if rel == "." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) || rel == ".." || filepath.IsAbs(rel) {
		return fmt.Errorf("stored path escapes attachment root")
	}
	return nil
}

func safePreviewImageMIME(value string) bool {
	switch strings.ToLower(strings.TrimSpace(value)) {
	case "image/png", "image/jpeg", "image/gif", "image/webp":
		return true
	default:
		return false
	}
}

func uniqueAttachmentFilename(dir, displayName string) string {
	displayName, err := safeAttachmentFilename(displayName)
	if err != nil {
		displayName = "attachment.bin"
	}
	ext := filepath.Ext(displayName)
	stem := strings.TrimSuffix(displayName, ext)
	for i := 0; i < 1000; i++ {
		candidate := displayName
		if i > 0 {
			candidate = fmt.Sprintf("%s-%d%s", stem, i, ext)
		}
		if _, err := os.Stat(filepath.Join(dir, candidate)); errors.Is(err, os.ErrNotExist) {
			return candidate
		}
	}
	return fmt.Sprintf("%s-%d%s", stem, time.Now().UnixNano(), ext)
}

func (s *Service) requireAccount(accountID string) error {
	_, err := s.getAccount(accountID)
	return err
}

func (s *Service) requireFolder(folderID string) error {
	var count int
	if err := s.db.QueryRow(`SELECT COUNT(*) FROM folders WHERE id = ?`, folderID).Scan(&count); err != nil {
		return err
	}
	if count == 0 {
		return fmt.Errorf("folder not found")
	}
	return nil
}

func (s *Service) remoteContentAllowed(accountID, messageID, senderEmail string) bool {
	accountID, err := validateID("account id", accountID)
	if err != nil {
		return false
	}
	messageID, err = validateID("message id", messageID)
	if err != nil {
		return false
	}
	targets := []struct {
		scope string
		id    string
	}{
		{scope: "message", id: messageID},
	}
	if email, err := validateEmail(senderEmail); err == nil {
		targets = append(targets, struct {
			scope string
			id    string
		}{scope: "sender", id: strings.ToLower(email)})
	}
	for _, target := range targets {
		var count int
		err := s.db.QueryRow(`
			SELECT COUNT(*)
			FROM remote_content_preferences
			WHERE account_id = ? AND scope = ? AND target_id = ?
		`, accountID, target.scope, target.id).Scan(&count)
		if err == nil && count > 0 {
			return true
		}
	}
	return false
}

func (s *Service) listFolders() ([]FolderView, error) {
	rows, err := s.db.Query(`
		SELECT id, account_id, name, COALESCE(display_name, name),
		       COALESCE(special_use, ''), unread_count
		FROM folders
		WHERE account_id IN (SELECT id FROM accounts WHERE provider_kind != 'removed')
		ORDER BY account_id,
			CASE special_use
				WHEN 'inbox' THEN 1
				WHEN 'sent' THEN 2
				WHEN 'drafts' THEN 3
				WHEN 'archive' THEN 4
				WHEN 'trash' THEN 5
				ELSE 10
			END, name
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var folders []FolderView
	for rows.Next() {
		var f FolderView
		if err := rows.Scan(&f.ID, &f.AccountID, &f.Name, &f.DisplayName, &f.SpecialUse, &f.UnreadCount); err != nil {
			return nil, err
		}
		folders = append(folders, f)
	}
	return folders, rows.Err()
}

func (s *Service) firstInboxFolderID() (string, error) {
	var id string
	err := s.db.QueryRow(`
		SELECT id
		FROM folders
		WHERE special_use = 'inbox'
		  AND account_id IN (SELECT id FROM accounts WHERE provider_kind != 'removed')
		ORDER BY CASE account_id WHEN 'demo-local' THEN 0 ELSE 1 END, account_id
		LIMIT 1
	`).Scan(&id)
	if err == sql.ErrNoRows {
		return "", nil
	}
	return id, err
}

func (s *Service) folderIDForSpecial(accountID, specialUse string) (string, error) {
	accountID, err := validateID("account id", accountID)
	if err != nil {
		return "", err
	}
	var id string
	err = s.db.QueryRow(`
		SELECT id FROM folders WHERE account_id = ? AND special_use = ? LIMIT 1
	`, accountID, specialUse).Scan(&id)
	if err == sql.ErrNoRows {
		return "", fmt.Errorf("%s folder not found", specialUse)
	}
	return id, err
}

func ensureDefaultFoldersTx(tx *sql.Tx, accountID string) error {
	folders := []struct {
		suffix, name, special string
	}{
		{"inbox", "Inbox", "inbox"},
		{"sent", "Sent", "sent"},
		{"drafts", "Drafts", "drafts"},
		{"archive", "Archive", "archive"},
		{"trash", "Trash", "trash"},
	}
	for _, folder := range folders {
		id := folder.suffix
		if accountID != "demo-local" {
			id = accountID + "-" + folder.suffix
		}
		if _, err := tx.Exec(`
			INSERT OR IGNORE INTO folders (id, account_id, name, display_name, special_use, unread_count)
			VALUES (?, ?, ?, ?, ?, 0)
		`, id, accountID, folder.name, folder.name, folder.special); err != nil {
			return err
		}
	}
	return nil
}

func scanMessageRows(rows *sql.Rows) ([]MessageRow, error) {
	var result []MessageRow
	for rows.Next() {
		var m MessageRow
		var isRead, isFlagged int
		if err := rows.Scan(
			&m.ID,
			&m.FolderID,
			&m.Subject,
			&m.SenderName,
			&m.SenderEmail,
			&m.DateReceived,
			&isRead,
			&isFlagged,
			&m.Snippet,
		); err != nil {
			return nil, err
		}
		m.IsRead = isRead == 1
		m.IsFlagged = isFlagged == 1
		result = append(result, m)
	}
	return result, rows.Err()
}

func boolToInt(b bool) int {
	if b {
		return 1
	}
	return 0
}

func accountIDForEmail(email string) string {
	return "acct-" + strings.ToLower(email)
}

func smtpClient(account accountRecord, tlsConfig *tls.Config) (*gomail.Client, error) {
	host, err := validateHost(account.SMTPHost)
	if err != nil {
		return nil, err
	}
	port, err := validatePort(account.SMTPPort)
	if err != nil {
		return nil, err
	}
	security, err := validateSecurity(account.SMTPSecurity)
	if err != nil {
		return nil, err
	}
	opts := []gomail.Option{gomail.WithPort(port)}
	if tlsConfig != nil {
		opts = append(opts, gomail.WithTLSConfig(tlsConfig))
	}
	if security == "tls" {
		opts = append(opts, gomail.WithSSL())
	} else {
		opts = append(opts, gomail.WithTLSPolicy(gomail.TLSMandatory))
	}
	return gomail.NewClient(host, opts...)
}

func (s *Service) smtpSecret(account accountRecord) (string, string, error) {
	cred, err := s.credentials.Get(account.ID)
	if err != nil {
		return "", "", fmt.Errorf("failed to load credential for send: %w", err)
	}
	authMode := authModeForProvider(account.ProviderKind)
	if authMode == authModeOAuth {
		updated, err := s.refreshOAuthCredentialIfNeeded(context.Background(), account.ID, cred)
		if err != nil {
			return "", "", err
		}
		if strings.TrimSpace(updated.AccessToken) == "" {
			return "", "", fmt.Errorf("OAuth access token is empty")
		}
		return updated.AccessToken, authModeOAuth, nil
	}
	if strings.TrimSpace(cred.Secret) == "" {
		return "", "", fmt.Errorf("account password is empty")
	}
	return cred.Secret, authModePassword, nil
}

func (s *Service) imapSecret(account accountRecord) (string, string, error) {
	cred, err := s.credentials.Get(account.ID)
	if err != nil {
		return "", "", fmt.Errorf("failed to load credential for sync: %w", err)
	}
	authMode := authModeForProvider(account.ProviderKind)
	if authMode == authModeOAuth {
		updated, err := s.refreshOAuthCredentialIfNeeded(context.Background(), account.ID, cred)
		if err != nil {
			return "", "", err
		}
		if strings.TrimSpace(updated.AccessToken) == "" {
			return "", "", fmt.Errorf("OAuth access token is empty")
		}
		return updated.AccessToken, authModeOAuth, nil
	}
	if strings.TrimSpace(cred.Secret) == "" {
		return "", "", fmt.Errorf("account password is empty")
	}
	return cred.Secret, authModePassword, nil
}

func (s *Service) loadSyncState(accountID, folderName string) (imap.SyncState, error) {
	accountID, err := validateID("account id", accountID)
	if err != nil {
		return imap.SyncState{}, err
	}
	folderName = strings.TrimSpace(folderName)
	if folderName == "" || len(folderName) > 120 || strings.ContainsAny(folderName, "\r\n") {
		return imap.SyncState{}, fmt.Errorf("folder name is invalid")
	}

	var state imap.SyncState
	err = s.db.QueryRow(`
		SELECT uid_validity, last_uid, highest_modseq
		FROM sync_state
		WHERE account_id = ? AND folder_name = ?
	`, accountID, folderName).Scan(&state.UIDValidity, &state.LastUID, &state.HighestModSeq)
	if err == sql.ErrNoRows {
		state.Limit = 50
		return state, nil
	}
	if err != nil {
		return imap.SyncState{}, err
	}
	state.Limit = 50
	return state, nil
}

func (s *Service) saveSyncState(accountID, folderName string, state imap.SyncState) error {
	accountID, err := validateID("account id", accountID)
	if err != nil {
		return err
	}
	folderName = strings.TrimSpace(folderName)
	if folderName == "" || len(folderName) > 120 || strings.ContainsAny(folderName, "\r\n") {
		return fmt.Errorf("folder name is invalid")
	}
	_, err = s.db.Exec(`
		INSERT INTO sync_state (account_id, folder_name, uid_validity, last_uid, highest_modseq, updated_at)
		VALUES (?, ?, ?, ?, ?, unixepoch())
		ON CONFLICT(account_id, folder_name) DO UPDATE SET
			uid_validity = excluded.uid_validity,
			last_uid = excluded.last_uid,
			highest_modseq = excluded.highest_modseq,
			updated_at = unixepoch()
	`, accountID, folderName, state.UIDValidity, state.LastUID, state.HighestModSeq)
	return err
}

func (s *Service) createLocalSentMessage(d Draft, from string) error {
	subject, err := validateSubject(d.Subject)
	if err != nil {
		return err
	}
	bodyHTML, err := validateHTMLBody(d.BodyHTML)
	if err != nil {
		return err
	}
	safe := sanitizeMessageHTML(bodyHTML)
	folderID, err := s.folderIDForSpecial(d.AccountID, "sent")
	if err != nil {
		return err
	}
	_, err = s.db.Exec(`
		INSERT OR REPLACE INTO messages
			(id, account_id, folder_id, subject, sender_email, date_received, is_read, body_html, body_text, snippet)
		VALUES (?, ?, ?, ?, ?, unixepoch(), 1, ?, ?, ?)
	`, "sent-"+d.ID, d.AccountID, folderID, subject, from, safe.HTML, safe.Text, subject)
	if err != nil {
		return err
	}
	_, _ = s.db.Exec(`DELETE FROM composer_drafts WHERE id = ?`, d.ID)
	return nil
}

func (s *Service) cacheSentMessage(account accountRecord, subject, bodyHTML, bodyText string) error {
	folderID, err := s.folderIDForSpecial(account.ID, "sent")
	if err != nil {
		return err
	}
	id := fmt.Sprintf("sent-%s-%d", account.ID, s.now().UnixNano())
	_, err = s.db.Exec(`
		INSERT INTO messages
			(id, account_id, folder_id, subject, sender_email, date_received, is_read, body_html, body_text, snippet)
		VALUES (?, ?, ?, ?, ?, unixepoch(), 1, ?, ?, ?)
	`, id, account.ID, folderID, subject, account.Email, bodyHTML, bodyText, subject)
	return err
}

func (s *Service) count(query string) (int, error) {
	var count int
	if err := s.db.QueryRow(query).Scan(&count); err != nil {
		return 0, err
	}
	return count, nil
}

func (s *Service) settingOr(key, fallback string) string {
	var value string
	if err := s.db.QueryRow(`SELECT COALESCE(value, '') FROM settings WHERE key = ?`, key).Scan(&value); err != nil || value == "" {
		return fallback
	}
	return value
}

func (s *Service) setSetting(key, value string) error {
	_, err := s.db.Exec(`
		INSERT OR REPLACE INTO settings (key, value)
		VALUES (?, ?)
	`, key, value)
	return err
}
