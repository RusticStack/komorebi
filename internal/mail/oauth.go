package mail

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"html"
	"io"
	"net"
	"net/http"
	"net/url"
	"strings"
	"time"

	"komorebi/internal/credentials"
)

const oauthPendingPrefix = "oauth:pending:"

type oauthCallbackSession struct {
	server  *http.Server
	addr    string
	result  OAuthCallbackStatusResponse
	started time.Time
}

type oauthPending struct {
	State        string `json:"state"`
	CodeVerifier string `json:"codeVerifier"`
	ProviderKind string `json:"providerKind"`
	Email        string `json:"email"`
	DisplayName  string `json:"displayName"`
	ClientID     string `json:"clientId"`
	RedirectURI  string `json:"redirectUri"`
	ExpiresAt    int64  `json:"expiresAt"`
}

type oauthTokenResponse struct {
	AccessToken  string `json:"access_token"`
	RefreshToken string `json:"refresh_token"`
	TokenType    string `json:"token_type"`
	ExpiresIn    int64  `json:"expires_in"`
	Scope        string `json:"scope"`
}

func (s *Service) StartOAuthAccountSetup(req OAuthStartRequest) (OAuthStartResponse, error) {
	provider, err := providerPolicyByKind(req.ProviderKind)
	if err != nil {
		return OAuthStartResponse{}, err
	}
	if provider.AuthMode != authModeOAuth {
		return OAuthStartResponse{}, fmt.Errorf("%s does not use OAuth setup", provider.Label)
	}

	email, err := validateEmail(req.Email)
	if err != nil {
		return OAuthStartResponse{}, err
	}
	displayName, err := validateDisplayName(req.DisplayName)
	if err != nil {
		return OAuthStartResponse{}, err
	}
	clientID, err := validateClientID(req.ClientID)
	if err != nil {
		return OAuthStartResponse{}, err
	}
	redirectURI, err := validateRedirectURI(req.RedirectURI)
	if err != nil {
		return OAuthStartResponse{}, err
	}

	state, err := randomURLToken(32)
	if err != nil {
		return OAuthStartResponse{}, err
	}
	verifier, err := randomURLToken(64)
	if err != nil {
		return OAuthStartResponse{}, err
	}
	challenge := pkceChallenge(verifier)
	expiresAt := s.now().Add(10 * time.Minute).Unix()

	pending := oauthPending{
		State:        state,
		CodeVerifier: verifier,
		ProviderKind: provider.ProviderKind,
		Email:        email,
		DisplayName:  displayName,
		ClientID:     clientID,
		RedirectURI:  redirectURI,
		ExpiresAt:    expiresAt,
	}
	if err := s.saveOAuthPending(pending); err != nil {
		return OAuthStartResponse{}, err
	}

	authURL, err := buildAuthURL(provider, clientID, redirectURI, state, challenge)
	if err != nil {
		return OAuthStartResponse{}, err
	}
	callbackMode := "manual"
	if err := s.startOAuthCallbackServer(pending); err == nil {
		callbackMode = "loopback"
	}

	return OAuthStartResponse{
		ProviderKind: provider.ProviderKind,
		AuthURL:      authURL,
		State:        state,
		RedirectURI:  redirectURI,
		Scopes:       append([]string(nil), provider.Scopes...),
		ExpiresAt:    expiresAt,
		CallbackMode: callbackMode,
	}, nil
}

func (s *Service) GetOAuthCallbackStatus(req OAuthCallbackStatusRequest) (OAuthCallbackStatusResponse, error) {
	state, err := validateID("oauth state", req.State)
	if err != nil {
		return OAuthCallbackStatusResponse{}, err
	}
	s.oauthCallbacksMu.Lock()
	defer s.oauthCallbacksMu.Unlock()
	session, ok := s.oauthCallbacks[state]
	if !ok {
		return OAuthCallbackStatusResponse{State: state}, nil
	}
	return session.result, nil
}

func (s *Service) CompleteOAuthAccountSetup(req OAuthCompleteRequest) (string, error) {
	state, err := validateID("oauth state", req.State)
	if err != nil {
		return "", err
	}
	code, err := validateOAuthCode(req.Code)
	if err != nil {
		return "", err
	}
	clientID, err := validateClientID(req.ClientID)
	if err != nil {
		return "", err
	}
	redirectURI, err := validateRedirectURI(req.RedirectURI)
	if err != nil {
		return "", err
	}

	pending, err := s.loadOAuthPending(state)
	if err != nil {
		return "", err
	}
	if s.now().Unix() > pending.ExpiresAt {
		_ = s.deleteOAuthPending(state)
		return "", fmt.Errorf("OAuth setup expired")
	}
	if pending.ClientID != clientID || pending.RedirectURI != redirectURI {
		return "", fmt.Errorf("OAuth callback does not match the pending setup")
	}

	provider, err := providerPolicyByKind(pending.ProviderKind)
	if err != nil {
		return "", err
	}
	token, err := s.exchangeOAuthToken(context.Background(), provider, pending, OAuthCompleteRequest{
		State:       state,
		Code:        code,
		ClientID:    clientID,
		RedirectURI: redirectURI,
	})
	if err != nil {
		return "", err
	}
	if strings.TrimSpace(token.AccessToken) == "" && strings.TrimSpace(token.RefreshToken) == "" {
		return "", fmt.Errorf("OAuth provider did not return a usable token")
	}

	accountID := accountIDForEmail(pending.Email)
	if err := s.insertOAuthAccount(accountID, pending, provider, token); err != nil {
		return "", err
	}
	_ = s.deleteOAuthPending(state)
	return accountID, nil
}

func (s *Service) startOAuthCallbackServer(pending oauthPending) error {
	parsed, err := url.Parse(pending.RedirectURI)
	if err != nil {
		return err
	}
	if parsed.Scheme != "http" || !isLoopbackHost(parsed.Hostname()) || parsed.Port() == "" {
		return fmt.Errorf("OAuth loopback requires an explicit localhost HTTP port")
	}
	path := parsed.EscapedPath()
	if path == "" {
		path = "/"
	}
	addr := net.JoinHostPort(parsed.Hostname(), parsed.Port())

	s.stopOAuthCallbacksForAddr(addr)
	listener, err := net.Listen("tcp", addr)
	if err != nil {
		return err
	}

	mux := http.NewServeMux()
	server := &http.Server{Handler: mux, ReadHeaderTimeout: 5 * time.Second}
	session := &oauthCallbackSession{
		server: server,
		addr:   addr,
		result: OAuthCallbackStatusResponse{
			State:     pending.State,
			ExpiresAt: pending.ExpiresAt,
		},
		started: s.now(),
	}
	s.oauthCallbacksMu.Lock()
	s.oauthCallbacks[pending.State] = session
	s.oauthCallbacksMu.Unlock()

	mux.HandleFunc(path, func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet {
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}
		query := r.URL.Query()
		if query.Get("state") != pending.State {
			s.finishOAuthCallback(pending.State, "", "OAuth callback state did not match")
			writeOAuthCallbackPage(w, "OAuth callback could not be verified.")
			go shutdownOAuthServer(server)
			return
		}
		if providerError := strings.TrimSpace(query.Get("error")); providerError != "" {
			s.finishOAuthCallback(pending.State, "", "OAuth provider returned: "+providerError)
			writeOAuthCallbackPage(w, "OAuth provider returned an error.")
			go shutdownOAuthServer(server)
			return
		}
		code, err := validateOAuthCode(query.Get("code"))
		if err != nil {
			s.finishOAuthCallback(pending.State, "", "OAuth authorization code was invalid")
			writeOAuthCallbackPage(w, "OAuth authorization code was invalid.")
			go shutdownOAuthServer(server)
			return
		}
		accountID, err := s.CompleteOAuthAccountSetup(OAuthCompleteRequest{
			State:       pending.State,
			Code:        code,
			ClientID:    pending.ClientID,
			RedirectURI: pending.RedirectURI,
		})
		if err != nil {
			s.finishOAuthCallback(pending.State, "", err.Error())
			writeOAuthCallbackPage(w, "Komorebi could not complete OAuth setup.")
			go shutdownOAuthServer(server)
			return
		}
		s.finishOAuthCallback(pending.State, accountID, "")
		writeOAuthCallbackPage(w, "OAuth setup complete. You can return to Komorebi.")
		go shutdownOAuthServer(server)
	})

	go func() {
		if err := server.Serve(listener); err != nil && err != http.ErrServerClosed {
			s.finishOAuthCallback(pending.State, "", "OAuth callback listener stopped")
		}
	}()
	go func() {
		wait := time.Until(time.Unix(pending.ExpiresAt, 0))
		if wait <= 0 {
			wait = time.Minute
		}
		timer := time.NewTimer(wait)
		defer timer.Stop()
		<-timer.C
		s.oauthCallbacksMu.Lock()
		session, ok := s.oauthCallbacks[pending.State]
		if ok && !session.result.Complete && session.result.Error == "" {
			session.result.Error = "OAuth setup expired"
		}
		s.oauthCallbacksMu.Unlock()
		shutdownOAuthServer(server)
	}()
	return nil
}

func (s *Service) stopOAuthCallbacksForAddr(addr string) {
	s.oauthCallbacksMu.Lock()
	var servers []*http.Server
	for _, session := range s.oauthCallbacks {
		if session.addr == addr && !session.result.Complete {
			servers = append(servers, session.server)
			session.result.Error = "OAuth callback listener was replaced"
		}
	}
	s.oauthCallbacksMu.Unlock()
	for _, server := range servers {
		shutdownOAuthServer(server)
	}
}

func (s *Service) finishOAuthCallback(state, accountID, message string) {
	s.oauthCallbacksMu.Lock()
	defer s.oauthCallbacksMu.Unlock()
	session, ok := s.oauthCallbacks[state]
	if !ok {
		return
	}
	session.result.Complete = accountID != ""
	session.result.AccountID = accountID
	session.result.Error = message
}

func isLoopbackHost(host string) bool {
	host = strings.ToLower(strings.TrimSpace(host))
	return host == "localhost" || host == "127.0.0.1" || host == "::1"
}

func shutdownOAuthServer(server *http.Server) {
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	_ = server.Shutdown(ctx)
}

func writeOAuthCallbackPage(w http.ResponseWriter, message string) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	_, _ = fmt.Fprintf(w, "<!doctype html><meta charset=\"utf-8\"><title>Komorebi OAuth</title><p>%s</p>", html.EscapeString(message))
}

func (s *Service) saveOAuthPending(pending oauthPending) error {
	encoded, err := json.Marshal(pending)
	if err != nil {
		return err
	}
	_, err = s.db.Exec(`
		INSERT OR REPLACE INTO settings (key, value)
		VALUES (?, ?)
	`, oauthPendingPrefix+pending.State, string(encoded))
	return err
}

func (s *Service) loadOAuthPending(state string) (oauthPending, error) {
	var raw string
	err := s.db.QueryRow(`SELECT COALESCE(value, '') FROM settings WHERE key = ?`, oauthPendingPrefix+state).Scan(&raw)
	if err == sql.ErrNoRows {
		return oauthPending{}, fmt.Errorf("OAuth setup was not found")
	}
	if err != nil {
		return oauthPending{}, err
	}
	var pending oauthPending
	if err := json.Unmarshal([]byte(raw), &pending); err != nil {
		return oauthPending{}, fmt.Errorf("OAuth setup is invalid")
	}
	return pending, nil
}

func (s *Service) deleteOAuthPending(state string) error {
	_, err := s.db.Exec(`DELETE FROM settings WHERE key = ?`, oauthPendingPrefix+state)
	return err
}

func (s *Service) exchangeOAuthTokenHTTP(ctx context.Context, provider providerPolicy, pending oauthPending, req OAuthCompleteRequest) (oauthTokenResponse, error) {
	form := url.Values{}
	form.Set("grant_type", "authorization_code")
	form.Set("code", req.Code)
	form.Set("client_id", req.ClientID)
	form.Set("redirect_uri", req.RedirectURI)
	form.Set("code_verifier", pending.CodeVerifier)

	httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, provider.TokenEndpoint, strings.NewReader(form.Encode()))
	if err != nil {
		return oauthTokenResponse{}, err
	}
	httpReq.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	httpReq.Header.Set("Accept", "application/json")

	client := s.httpClient
	if client == nil {
		client = &http.Client{Timeout: 20 * time.Second}
	}
	resp, err := client.Do(httpReq)
	if err != nil {
		return oauthTokenResponse{}, fmt.Errorf("OAuth token exchange failed: %w", err)
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if err != nil {
		return oauthTokenResponse{}, err
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return oauthTokenResponse{}, fmt.Errorf("OAuth token exchange returned HTTP %d", resp.StatusCode)
	}

	var token oauthTokenResponse
	if err := json.Unmarshal(body, &token); err != nil {
		return oauthTokenResponse{}, fmt.Errorf("OAuth token response is invalid")
	}
	return token, nil
}

func (s *Service) refreshOAuthCredentialIfNeeded(ctx context.Context, accountID string, cred credentials.CredentialData) (credentials.CredentialData, error) {
	if strings.TrimSpace(cred.AccessToken) != "" && (cred.Expiry.IsZero() || s.now().Before(cred.Expiry.Add(-1*time.Minute))) {
		return cred, nil
	}
	if strings.TrimSpace(cred.RefreshToken) == "" {
		return credentials.CredentialData{}, fmt.Errorf("OAuth refresh token is missing")
	}
	if strings.TrimSpace(cred.ClientID) == "" || strings.TrimSpace(cred.TokenEndpoint) == "" {
		return credentials.CredentialData{}, fmt.Errorf("OAuth credential metadata is incomplete")
	}

	form := url.Values{}
	form.Set("grant_type", "refresh_token")
	form.Set("refresh_token", cred.RefreshToken)
	form.Set("client_id", cred.ClientID)

	httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, cred.TokenEndpoint, strings.NewReader(form.Encode()))
	if err != nil {
		return credentials.CredentialData{}, err
	}
	httpReq.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	httpReq.Header.Set("Accept", "application/json")

	client := s.httpClient
	if client == nil {
		client = &http.Client{Timeout: 20 * time.Second}
	}
	resp, err := client.Do(httpReq)
	if err != nil {
		return credentials.CredentialData{}, fmt.Errorf("OAuth token refresh failed: %w", err)
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if err != nil {
		return credentials.CredentialData{}, err
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return credentials.CredentialData{}, fmt.Errorf("OAuth token refresh returned HTTP %d", resp.StatusCode)
	}

	var token oauthTokenResponse
	if err := json.Unmarshal(body, &token); err != nil {
		return credentials.CredentialData{}, fmt.Errorf("OAuth refresh response is invalid")
	}
	if strings.TrimSpace(token.AccessToken) == "" {
		return credentials.CredentialData{}, fmt.Errorf("OAuth refresh response did not include an access token")
	}
	cred.AccessToken = token.AccessToken
	if strings.TrimSpace(token.RefreshToken) != "" {
		cred.RefreshToken = token.RefreshToken
	}
	if token.ExpiresIn > 0 {
		cred.Expiry = s.now().Add(time.Duration(token.ExpiresIn) * time.Second)
	}
	if err := s.credentials.Save(accountID, cred); err != nil {
		return credentials.CredentialData{}, fmt.Errorf("failed to update OAuth credential securely: %w", err)
	}
	return cred, nil
}

func (s *Service) insertOAuthAccount(accountID string, pending oauthPending, provider providerPolicy, token oauthTokenResponse) error {
	tx, err := s.db.Begin()
	if err != nil {
		return err
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
	`, accountID, pending.Email, pending.DisplayName, provider.ProviderKind, pending.Email,
		provider.IMAPHost, provider.IMAPPort, provider.IMAPSecurity,
		provider.SMTPHost, provider.SMTPPort, provider.SMTPSecurity)
	if err != nil {
		return err
	}

	if err := ensureDefaultFoldersTx(tx, accountID); err != nil {
		return err
	}

	expiry := time.Time{}
	if token.ExpiresIn > 0 {
		expiry = s.now().Add(time.Duration(token.ExpiresIn) * time.Second)
	}
	if err := s.credentials.Save(accountID, credentials.CredentialData{
		AuthMode:      authModeOAuth,
		ProviderKind:  provider.ProviderKind,
		Username:      pending.Email,
		AccessToken:   token.AccessToken,
		RefreshToken:  token.RefreshToken,
		TokenEndpoint: provider.TokenEndpoint,
		ClientID:      pending.ClientID,
		Expiry:        expiry,
	}); err != nil {
		return fmt.Errorf("failed to store OAuth token securely: %w", err)
	}

	return tx.Commit()
}

func buildAuthURL(provider providerPolicy, clientID, redirectURI, state, challenge string) (string, error) {
	parsed, err := url.Parse(provider.AuthEndpoint)
	if err != nil {
		return "", err
	}
	query := parsed.Query()
	query.Set("client_id", clientID)
	query.Set("redirect_uri", redirectURI)
	query.Set("response_type", "code")
	query.Set("scope", strings.Join(provider.Scopes, " "))
	query.Set("state", state)
	query.Set("code_challenge", challenge)
	query.Set("code_challenge_method", "S256")
	query.Set("access_type", "offline")
	query.Set("prompt", "consent")
	parsed.RawQuery = query.Encode()
	return parsed.String(), nil
}

func randomURLToken(size int) (string, error) {
	raw := make([]byte, size)
	if _, err := rand.Read(raw); err != nil {
		return "", err
	}
	return base64.RawURLEncoding.EncodeToString(raw), nil
}

func pkceChallenge(verifier string) string {
	sum := sha256.Sum256([]byte(verifier))
	return base64.RawURLEncoding.EncodeToString(sum[:])
}
