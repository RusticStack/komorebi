package mail

// MailShellData is the data returned to the frontend for the primary shell.
type MailShellData struct {
	Accounts []AccountView `json:"accounts"`
	Folders  []FolderView  `json:"folders"`
	Messages []MessageRow  `json:"messages"`
	HasMore  bool          `json:"hasMore"`
	Total    int           `json:"total"`
}

type AccountView struct {
	ID           string `json:"id"`
	Email        string `json:"email"`
	DisplayName  string `json:"displayName"`
	Provider     string `json:"provider"`
	AuthMode     string `json:"authMode"`
	IMAPHost     string `json:"imapHost"`
	IMAPPort     int    `json:"imapPort"`
	IMAPSecurity string `json:"imapSecurity"`
	SMTPHost     string `json:"smtpHost"`
	SMTPPort     int    `json:"smtpPort"`
	SMTPSecurity string `json:"smtpSecurity"`
	Safety       string `json:"safety"`
}

type FolderView struct {
	ID          string `json:"id"`
	AccountID   string `json:"accountId"`
	Name        string `json:"name"`
	DisplayName string `json:"displayName"`
	SpecialUse  string `json:"specialUse"`
	UnreadCount int    `json:"unreadCount"`
}

type MessageRow struct {
	ID           string `json:"id"`
	FolderID     string `json:"folderId"`
	Subject      string `json:"subject"`
	SenderName   string `json:"senderName"`
	SenderEmail  string `json:"senderEmail"`
	DateReceived int64  `json:"dateReceived"`
	IsRead       bool   `json:"isRead"`
	IsFlagged    bool   `json:"isFlagged"`
	Snippet      string `json:"snippet"`
}

type MessagePageRequest struct {
	FolderID string `json:"folderId"`
	Cursor   int64  `json:"cursor"`
	Limit    int    `json:"limit"`
}

type MessagePageResponse struct {
	Messages   []MessageRow `json:"messages"`
	NextCursor int64        `json:"nextCursor"`
	HasMore    bool         `json:"hasMore"`
}

type MarkReadRequest struct {
	MessageID string `json:"messageId"`
	Read      bool   `json:"read"`
}

type SearchRequest struct {
	Query string `json:"query"`
	Limit int    `json:"limit"`
}

type MessageDetailRequest struct {
	MessageID string `json:"messageId"`
}

type MessageDetail struct {
	ID                   string `json:"id"`
	AccountID            string `json:"accountId"`
	FolderID             string `json:"folderId"`
	Subject              string `json:"subject"`
	SenderName           string `json:"senderName"`
	SenderEmail          string `json:"senderEmail"`
	DateReceived         int64  `json:"dateReceived"`
	SafeHTML             string `json:"safeHtml"`
	BodyText             string `json:"bodyText"`
	RenderMode           string `json:"renderMode"`
	RemoteContentBlocked int    `json:"remoteContentBlocked"`
	RemoteContentAllowed bool   `json:"remoteContentAllowed"`
	UnsafeHTMLRemoved    bool   `json:"unsafeHtmlRemoved"`
}

// Draft represents a persisted composer draft.
type Draft struct {
	ID        string `json:"id"`
	AccountID string `json:"accountId"`
	Subject   string `json:"subject"`
	To        string `json:"to"`
	BodyHTML  string `json:"bodyHtml"`
	UpdatedAt int64  `json:"updatedAt"`
}

// Account is the full account model returned to safe frontend surfaces.
type Account struct {
	ID           string `json:"id"`
	Email        string `json:"email"`
	DisplayName  string `json:"displayName"`
	ProviderKind string `json:"providerKind"`
	AuthMode     string `json:"authMode"`
	IMAPHost     string `json:"imapHost"`
	IMAPPort     int    `json:"imapPort"`
	IMAPSecurity string `json:"imapSecurity"`
	SMTPHost     string `json:"smtpHost"`
	SMTPPort     int    `json:"smtpPort"`
	SMTPSecurity string `json:"smtpSecurity"`
}

type AddAccountRequest struct {
	Email        string `json:"email"`
	DisplayName  string `json:"displayName"`
	ProviderKind string `json:"providerKind"`
	AuthMode     string `json:"authMode"`
	Password     string `json:"password"`
	IMAPHost     string `json:"imapHost"`
	IMAPPort     int    `json:"imapPort"`
	IMAPSecurity string `json:"imapSecurity"`
	SMTPHost     string `json:"smtpHost"`
	SMTPPort     int    `json:"smtpPort"`
	SMTPSecurity string `json:"smtpSecurity"`
}

type AccountSetupValidationRequest struct {
	Email        string `json:"email"`
	DisplayName  string `json:"displayName"`
	ProviderKind string `json:"providerKind"`
	AuthMode     string `json:"authMode"`
	Password     string `json:"password"`
	IMAPHost     string `json:"imapHost"`
	IMAPPort     int    `json:"imapPort"`
	IMAPSecurity string `json:"imapSecurity"`
	SMTPHost     string `json:"smtpHost"`
	SMTPPort     int    `json:"smtpPort"`
	SMTPSecurity string `json:"smtpSecurity"`
}

type AccountSetupValidationResponse struct {
	OK     bool              `json:"ok"`
	Checks []ValidationCheck `json:"checks"`
}

type ValidationCheck struct {
	ID     string `json:"id"`
	Label  string `json:"label"`
	Status string `json:"status"`
	Detail string `json:"detail"`
}

type DiscoverAccountSettingsRequest struct {
	Email string `json:"email"`
}

type DiscoveredAccountSettings struct {
	ProviderKind  string   `json:"providerKind"`
	ProviderLabel string   `json:"providerLabel"`
	Source        string   `json:"source"`
	Email         string   `json:"email"`
	Domain        string   `json:"domain"`
	AuthMode      string   `json:"authMode"`
	IMAPHost      string   `json:"imapHost"`
	IMAPPort      int      `json:"imapPort"`
	IMAPSecurity  string   `json:"imapSecurity"`
	SMTPHost      string   `json:"smtpHost"`
	SMTPPort      int      `json:"smtpPort"`
	SMTPSecurity  string   `json:"smtpSecurity"`
	Confidence    int      `json:"confidence"`
	Warnings      []string `json:"warnings"`
}

type DiscoveryDiagnostic struct {
	Source string `json:"source"`
	Status string `json:"status"`
	Detail string `json:"detail"`
}

type DiscoverAccountSettingsResponse struct {
	Email       string                      `json:"email"`
	Domain      string                      `json:"domain"`
	Candidates  []DiscoveredAccountSettings `json:"candidates"`
	Diagnostics []DiscoveryDiagnostic       `json:"diagnostics"`
}

type RemoveAccountRequest struct {
	AccountID   string `json:"accountId"`
	DeleteCache bool   `json:"deleteCache"`
}

type SendEmailRequest struct {
	AccountID string   `json:"accountId"`
	From      string   `json:"from"`
	To        []string `json:"to"`
	Subject   string   `json:"subject"`
	BodyHTML  string   `json:"bodyHtml"`
}

type SyncAccountRequest struct {
	AccountID    string `json:"accountId"`
	FolderPolicy string `json:"folderPolicy"`
}

type BackgroundSyncRequest struct {
	AccountID       string `json:"accountId"`
	IntervalSeconds int    `json:"intervalSeconds"`
	FolderPolicy    string `json:"folderPolicy"`
}

type BackgroundSyncStatus struct {
	AccountID       string   `json:"accountId"`
	Running         bool     `json:"running"`
	LastRun         int64    `json:"lastRun"`
	LastError       string   `json:"lastError"`
	IntervalSeconds int      `json:"intervalSeconds"`
	FolderPolicy    string   `json:"folderPolicy"`
	LastFolders     []string `json:"lastFolders"`
}

type IMAPTestRequest struct {
	AccountID string `json:"accountId"`
	Host      string `json:"host"`
	Port      int    `json:"port"`
	Username  string `json:"username"`
	Password  string `json:"password"`
	Security  string `json:"security"`
}

type StageAttachmentRequest struct {
	AccountID   string `json:"accountId"`
	MessageID   string `json:"messageId"`
	DisplayName string `json:"displayName"`
	Mime        string `json:"mime"`
	Data        []byte `json:"data"`
}

type AttachmentStageResult struct {
	ID          string `json:"id"`
	DisplayName string `json:"displayName"`
	SizeBytes   int    `json:"sizeBytes"`
}

type AttachmentView struct {
	ID          string `json:"id"`
	MessageID   string `json:"messageId"`
	DisplayName string `json:"displayName"`
	Mime        string `json:"mime"`
	SizeBytes   int    `json:"sizeBytes"`
	CreatedAt   int64  `json:"createdAt"`
}

type AttachmentRequest struct {
	AttachmentID string `json:"attachmentId"`
}

type AttachmentPreview struct {
	ID          string `json:"id"`
	DisplayName string `json:"displayName"`
	Mime        string `json:"mime"`
	Kind        string `json:"kind"`
	Text        string `json:"text"`
	DataURL     string `json:"dataUrl"`
	SizeBytes   int    `json:"sizeBytes"`
}

type AttachmentSaveResult struct {
	ID          string `json:"id"`
	DisplayName string `json:"displayName"`
	SavedPath   string `json:"savedPath"`
}

type RemoteContentPreferenceRequest struct {
	MessageID string `json:"messageId"`
	Scope     string `json:"scope"`
	Allowed   bool   `json:"allowed"`
}

type OpenExternalLinkRequest struct {
	URL string `json:"url"`
}

type OAuthStartRequest struct {
	ProviderKind string `json:"providerKind"`
	Email        string `json:"email"`
	DisplayName  string `json:"displayName"`
	ClientID     string `json:"clientId"`
	RedirectURI  string `json:"redirectUri"`
}

type OAuthStartResponse struct {
	ProviderKind string   `json:"providerKind"`
	AuthURL      string   `json:"authUrl"`
	State        string   `json:"state"`
	RedirectURI  string   `json:"redirectUri"`
	Scopes       []string `json:"scopes"`
	ExpiresAt    int64    `json:"expiresAt"`
	CallbackMode string   `json:"callbackMode"`
}

type OAuthCompleteRequest struct {
	State       string `json:"state"`
	Code        string `json:"code"`
	ClientID    string `json:"clientId"`
	RedirectURI string `json:"redirectUri"`
}

type OAuthCallbackStatusRequest struct {
	State string `json:"state"`
}

type OAuthCallbackStatusResponse struct {
	State     string `json:"state"`
	Complete  bool   `json:"complete"`
	AccountID string `json:"accountId"`
	Error     string `json:"error"`
	ExpiresAt int64  `json:"expiresAt"`
}

type ProviderPolicyView struct {
	ProviderKind string   `json:"providerKind"`
	Label        string   `json:"label"`
	AuthMode     string   `json:"authMode"`
	IMAPHost     string   `json:"imapHost"`
	IMAPPort     int      `json:"imapPort"`
	IMAPSecurity string   `json:"imapSecurity"`
	SMTPHost     string   `json:"smtpHost"`
	SMTPPort     int      `json:"smtpPort"`
	SMTPSecurity string   `json:"smtpSecurity"`
	Scopes       []string `json:"scopes"`
	Notes        []string `json:"notes"`
}

type DiagnosticsHealth struct {
	SchemaOK        bool              `json:"schemaOk"`
	AccountCount    int               `json:"accountCount"`
	MessageCount    int               `json:"messageCount"`
	DraftCount      int               `json:"draftCount"`
	AttachmentCount int               `json:"attachmentCount"`
	PendingOAuth    int               `json:"pendingOAuth"`
	ProviderCounts  map[string]int    `json:"providerCounts"`
	Checks          []DiagnosticCheck `json:"checks"`
	GeneratedAt     int64             `json:"generatedAt"`
}

type DiagnosticsExportRequest struct {
	IncludeSchema bool `json:"includeSchema"`
}

type DiagnosticsExportResult struct {
	Path        string `json:"path"`
	SizeBytes   int    `json:"sizeBytes"`
	GeneratedAt int64  `json:"generatedAt"`
}

type DiagnosticCheck struct {
	ID     string `json:"id"`
	Label  string `json:"label"`
	Status string `json:"status"`
	Detail string `json:"detail"`
}

type UIPreferences struct {
	Language string `json:"language"`
	Density  string `json:"density"`
}

type UIPreferencesRequest struct {
	Language string `json:"language"`
	Density  string `json:"density"`
}
