export interface AccountView {
  id: string
  email: string
  displayName: string
  provider: string
  authMode: string
  imapHost: string
  imapPort: number
  imapSecurity: string
  smtpHost: string
  smtpPort: number
  smtpSecurity: string
  safety: string
}

export interface FolderView {
  id: string
  accountId: string
  name: string
  displayName: string
  specialUse: string
  unreadCount: number
}

export interface MessageRow {
  id: string
  folderId: string
  subject: string
  senderName: string
  senderEmail: string
  dateReceived: number
  isRead: boolean
  isFlagged: boolean
  snippet: string
}

export interface MailShellData {
  accounts: AccountView[]
  folders: FolderView[]
  messages: MessageRow[]
  hasMore: boolean
  total: number
}

export interface Draft {
  id: string
  accountId: string
  subject: string
  to: string
  bodyHtml: string
  updatedAt: number
}

export interface MessagePageResponse {
  messages: MessageRow[]
  nextCursor: number
  hasMore: boolean
}

export interface MessageDetail {
  id: string
  accountId: string
  folderId: string
  subject: string
  senderName: string
  senderEmail: string
  dateReceived: number
  safeHtml: string
  bodyText: string
  renderMode: string
  remoteContentBlocked: number
  remoteContentAllowed: boolean
  unsafeHtmlRemoved: boolean
}

export interface ProviderPolicy {
  providerKind: string
  label: string
  authMode: string
  imapHost: string
  imapPort: number
  imapSecurity: string
  smtpHost: string
  smtpPort: number
  smtpSecurity: string
  scopes: string[]
  notes: string[]
}

export interface DiscoveredAccountSettings {
  providerKind: string
  providerLabel: string
  source: string
  email: string
  domain: string
  authMode: string
  imapHost: string
  imapPort: number
  imapSecurity: string
  smtpHost: string
  smtpPort: number
  smtpSecurity: string
  confidence: number
  warnings: string[]
}

export interface DiscoveryDiagnostic {
  source: string
  status: string
  detail: string
}

export interface DiscoverAccountSettingsResponse {
  email: string
  domain: string
  candidates: DiscoveredAccountSettings[]
  diagnostics: DiscoveryDiagnostic[]
}

export interface ValidationCheck {
  id: string
  label: string
  status: string
  detail: string
}

export interface AccountSetupValidationResponse {
  ok: boolean
  checks: ValidationCheck[]
}

export interface DiagnosticCheck {
  id: string
  label: string
  status: string
  detail: string
}

export interface DiagnosticsHealth {
  schemaOk: boolean
  accountCount: number
  messageCount: number
  draftCount: number
  attachmentCount: number
  pendingOAuth: number
  providerCounts: Record<string, number>
  checks: DiagnosticCheck[]
  generatedAt: number
}

export interface UIPreferences {
  language: 'en' | 'pt-PT'
  density: 'comfortable' | 'compact'
}

export interface OAuthStartResponse {
  providerKind: string
  authUrl: string
  state: string
  redirectUri: string
  scopes: string[]
  expiresAt: number
  callbackMode: 'loopback' | 'manual'
}

export interface OAuthCallbackStatusResponse {
  state: string
  complete: boolean
  accountId: string
  error: string
  expiresAt: number
}

export interface BackgroundSyncStatus {
  accountId: string
  running: boolean
  lastRun: number
  lastError: string
  intervalSeconds: number
  folderPolicy: string
  lastFolders: string[]
}

export interface AttachmentView {
  id: string
  messageId: string
  displayName: string
  mime: string
  sizeBytes: number
  createdAt: number
}

export interface AttachmentPreview {
  id: string
  displayName: string
  mime: string
  kind: 'text' | 'image' | 'unsupported'
  text: string
  dataUrl: string
  sizeBytes: number
}

export interface AttachmentSaveResult {
  id: string
  displayName: string
  savedPath: string
}

export interface DiagnosticsExportResult {
  path: string
  sizeBytes: number
  generatedAt: number
}
