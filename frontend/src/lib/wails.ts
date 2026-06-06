import {
  AccountSetupValidationResponse,
  AttachmentPreview,
  AttachmentSaveResult,
  AttachmentView,
  BackgroundSyncStatus,
  DiscoverAccountSettingsResponse,
  DiagnosticsExportResult,
  DiagnosticsHealth,
  MailShellData,
  MessageDetail,
  MessagePageResponse,
  OAuthCallbackStatusResponse,
  OAuthStartResponse,
  ProviderPolicy,
  UIPreferences,
} from '../types/mail'

declare global {
  interface Window {
    go?: {
      mail?: {
        Service?: {
          LoadMailShell: () => Promise<MailShellData>
          LoadMessagePage: (req: { folderId: string; cursor: number; limit: number }) => Promise<MessagePageResponse>
          LoadMessageDetail: (req: { messageId: string }) => Promise<MessageDetail>
          MarkRead: (req: { messageId: string; read: boolean }) => Promise<void>
          SearchMail: (req: { query: string; limit: number }) => Promise<any>
          ListDrafts: () => Promise<any>
          SaveDraft: (draft: any) => Promise<void>
          DeleteDraft: (id: string) => Promise<void>
          SendDraft: (id: string) => Promise<void>
          AddAccount: (req: any) => Promise<string>
          ValidateAccountSetup: (req: any) => Promise<AccountSetupValidationResponse>
          DiscoverAccountSettings: (req: { email: string }) => Promise<DiscoverAccountSettingsResponse>
          ListAccounts: () => Promise<any>
          RemoveAccount: (req: { accountId: string; deleteCache: boolean }) => Promise<void>
          SyncAccount: (req: { accountId: string }) => Promise<void>
          StartBackgroundSync: (req: { accountId: string; intervalSeconds: number; folderPolicy?: string }) => Promise<BackgroundSyncStatus>
          StopBackgroundSync: (req: { accountId: string }) => Promise<void>
          GetBackgroundSyncStatus: () => Promise<BackgroundSyncStatus[]>
          TestAndSyncIMAP: (req: any) => Promise<void>
          StageAttachment: (req: any) => Promise<any>
          ListAttachments: (req: { messageId: string }) => Promise<AttachmentView[]>
          PreviewAttachment: (req: { attachmentId: string }) => Promise<AttachmentPreview>
          SaveAttachment: (req: { attachmentId: string }) => Promise<AttachmentSaveResult>
          DeleteAttachment: (req: { attachmentId: string }) => Promise<void>
          SetRemoteContentPreference: (req: { messageId: string; scope: string; allowed: boolean }) => Promise<void>
          OpenExternalLink: (req: { url: string }) => Promise<void>
          StartOAuthAccountSetup: (req: any) => Promise<OAuthStartResponse>
          CompleteOAuthAccountSetup: (req: any) => Promise<string>
          GetOAuthCallbackStatus: (req: { state: string }) => Promise<OAuthCallbackStatusResponse>
          ListProviderPolicies: () => Promise<ProviderPolicy[]>
          GetDiagnosticsHealth: () => Promise<DiagnosticsHealth>
          ExportDiagnostics: (req: { includeSchema: boolean }) => Promise<DiagnosticsExportResult>
          GetUIPreferences: () => Promise<UIPreferences>
          UpdateUIPreferences: (req: Partial<UIPreferences>) => Promise<UIPreferences>
        }
      }
    }
  }
}

const fallbackShell: MailShellData = {
  accounts: [{
    id: 'demo-local',
    email: 'you@local.test',
    displayName: 'Local Demo',
    provider: 'local',
    authMode: 'local',
    imapHost: '',
    imapPort: 0,
    imapSecurity: 'tls',
    smtpHost: '',
    smtpPort: 0,
    smtpSecurity: 'starttls',
    safety: 'local-only',
  }],
  folders: [
    { id: 'inbox', accountId: 'demo-local', name: 'Inbox', displayName: 'Inbox', specialUse: 'inbox', unreadCount: 14 },
    { id: 'sent', accountId: 'demo-local', name: 'Sent', displayName: 'Sent', specialUse: 'sent', unreadCount: 0 },
    { id: 'drafts', accountId: 'demo-local', name: 'Drafts', displayName: 'Drafts', specialUse: 'drafts', unreadCount: 0 },
  ],
  messages: [],
  hasMore: true,
  total: 48,
}

const fallbackPolicies: ProviderPolicy[] = [
  {
    providerKind: 'gmail',
    label: 'Gmail',
    authMode: 'oauth',
    imapHost: 'imap.gmail.com',
    imapPort: 993,
    imapSecurity: 'tls',
    smtpHost: 'smtp.gmail.com',
    smtpPort: 587,
    smtpSecurity: 'starttls',
    scopes: ['openid', 'email', 'profile', 'https://mail.google.com/'],
    notes: [],
  },
  {
    providerKind: 'microsoft',
    label: 'Microsoft 365 / Outlook',
    authMode: 'oauth',
    imapHost: 'outlook.office365.com',
    imapPort: 993,
    imapSecurity: 'tls',
    smtpHost: 'smtp.office365.com',
    smtpPort: 587,
    smtpSecurity: 'starttls',
    scopes: ['offline_access', 'openid', 'email', 'profile'],
    notes: [],
  },
  {
    providerKind: 'generic',
    label: 'Generic IMAP/SMTP',
    authMode: 'password',
    imapHost: '',
    imapPort: 993,
    imapSecurity: 'tls',
    smtpHost: '',
    smtpPort: 587,
    smtpSecurity: 'starttls',
    scopes: [],
    notes: [],
  },
]

function fallbackDiscovery(email: string): DiscoverAccountSettingsResponse {
  const domain = email.includes('@') ? email.split('@').pop() ?? '' : ''
  return {
    email,
    domain,
    candidates: [
      {
        providerKind: 'generic',
        providerLabel: 'Common hosting pattern',
        source: 'fallback',
        email,
        domain,
        authMode: 'password',
        imapHost: domain ? `mail.${domain}` : '',
        imapPort: 993,
        imapSecurity: 'tls',
        smtpHost: domain ? `mail.${domain}` : '',
        smtpPort: 465,
        smtpSecurity: 'tls',
        confidence: 40,
        warnings: ['Fallback only; confirm these hostnames with the provider before saving.'],
      },
    ],
    diagnostics: [],
  }
}

export function useWails() {
  const svc = window.go?.mail?.Service

  return {
    async LoadMailShell() {
      if (!svc?.LoadMailShell) return fallbackShell
      return await svc.LoadMailShell()
    },

    async LoadMessagePage(req: { folderId: string; cursor: number; limit: number }) {
      if (!svc?.LoadMessagePage) return { messages: [], nextCursor: 0, hasMore: false }
      return await svc.LoadMessagePage(req)
    },

    async LoadMessageDetail(req: { messageId: string }) {
      if (!svc?.LoadMessageDetail) {
        return {
          id: req.messageId,
          accountId: 'demo-local',
          folderId: 'inbox',
          subject: '',
          senderName: '',
          senderEmail: '',
          dateReceived: 0,
          safeHtml: '<p>Select a seeded message in the Wails app.</p>',
          bodyText: '',
          renderMode: 'plain-text',
          remoteContentBlocked: 0,
          remoteContentAllowed: false,
          unsafeHtmlRemoved: false,
        }
      }
      return await svc.LoadMessageDetail(req)
    },

    async SearchMail(req: { query: string; limit: number }) {
      if (!svc?.SearchMail) return []
      return await svc.SearchMail(req)
    },

    async MarkRead(req: { messageId: string; read: boolean }) {
      if (!svc?.MarkRead) return
      await svc.MarkRead(req)
    },

    async ListDrafts() {
      if (!svc?.ListDrafts) return []
      return await svc.ListDrafts()
    },

    async SaveDraft(draft: any) {
      if (!svc?.SaveDraft) return
      await svc.SaveDraft(draft)
    },

    async DeleteDraft(id: string) {
      if (!svc?.DeleteDraft) return
      await svc.DeleteDraft(id)
    },

    async SendDraft(id: string) {
      if (!svc?.SendDraft) return
      await svc.SendDraft(id)
    },

    async AddAccount(req: any) {
      if (!svc?.AddAccount) return 'demo-local'
      return await svc.AddAccount(req)
    },

    async ValidateAccountSetup(req: any) {
      if (!svc?.ValidateAccountSetup) {
        return {
          ok: false,
          checks: [
            {
              id: 'runtime',
              label: 'Runtime',
              status: 'warn',
              detail: 'Account validation is only available inside the Wails desktop app.',
            },
          ],
        } as AccountSetupValidationResponse
      }
      return await svc.ValidateAccountSetup(req)
    },

    async DiscoverAccountSettings(req: { email: string }) {
      if (!svc?.DiscoverAccountSettings) return fallbackDiscovery(req.email)
      return await svc.DiscoverAccountSettings(req)
    },

    async ListAccounts() {
      if (!svc?.ListAccounts) return fallbackShell.accounts
      return await svc.ListAccounts()
    },

    async RemoveAccount(req: { accountId: string; deleteCache: boolean }) {
      if (!svc?.RemoveAccount) return
      await svc.RemoveAccount(req)
    },

    async SyncAccount(req: { accountId: string }) {
      if (!svc?.SyncAccount) return
      await svc.SyncAccount(req)
    },

    async StartBackgroundSync(req: { accountId: string; intervalSeconds: number; folderPolicy?: string }) {
      if (!svc?.StartBackgroundSync) {
        return {
          accountId: req.accountId,
          running: false,
          lastRun: 0,
          lastError: 'Background sync is only available inside the Wails desktop app.',
          intervalSeconds: req.intervalSeconds,
          folderPolicy: 'special',
          lastFolders: [],
        } as BackgroundSyncStatus
      }
      return await svc.StartBackgroundSync(req)
    },

    async StopBackgroundSync(req: { accountId: string }) {
      if (!svc?.StopBackgroundSync) return
      await svc.StopBackgroundSync(req)
    },

    async GetBackgroundSyncStatus() {
      if (!svc?.GetBackgroundSyncStatus) return [] as BackgroundSyncStatus[]
      return await svc.GetBackgroundSyncStatus()
    },

    async TestAndSyncIMAP(req: any) {
      if (!svc?.TestAndSyncIMAP) return
      await svc.TestAndSyncIMAP(req)
    },

    async StageAttachment(req: any) {
      if (!svc?.StageAttachment) return null
      return await svc.StageAttachment(req)
    },

    async ListAttachments(req: { messageId: string }) {
      if (!svc?.ListAttachments) return [] as AttachmentView[]
      return await svc.ListAttachments(req)
    },

    async PreviewAttachment(req: { attachmentId: string }) {
      if (!svc?.PreviewAttachment) {
        return {
          id: req.attachmentId,
          displayName: '',
          mime: 'application/octet-stream',
          kind: 'unsupported',
          text: '',
          dataUrl: '',
          sizeBytes: 0,
        } as AttachmentPreview
      }
      return await svc.PreviewAttachment(req)
    },

    async SaveAttachment(req: { attachmentId: string }) {
      if (!svc?.SaveAttachment) return { id: req.attachmentId, displayName: '', savedPath: '' } as AttachmentSaveResult
      return await svc.SaveAttachment(req)
    },

    async DeleteAttachment(req: { attachmentId: string }) {
      if (!svc?.DeleteAttachment) return
      await svc.DeleteAttachment(req)
    },

    async SetRemoteContentPreference(req: { messageId: string; scope: string; allowed: boolean }) {
      if (!svc?.SetRemoteContentPreference) return
      await svc.SetRemoteContentPreference(req)
    },

    async OpenExternalLink(req: { url: string }) {
      if (!svc?.OpenExternalLink) {
        window.open(req.url, '_blank', 'noopener,noreferrer')
        return
      }
      await svc.OpenExternalLink(req)
    },

    async StartOAuthAccountSetup(req: any) {
      if (!svc?.StartOAuthAccountSetup) {
        return {
          providerKind: req.providerKind,
          authUrl: '',
          state: '',
          redirectUri: req.redirectUri,
          scopes: [],
          expiresAt: 0,
          callbackMode: 'manual',
        } as OAuthStartResponse
      }
      return await svc.StartOAuthAccountSetup(req)
    },

    async CompleteOAuthAccountSetup(req: any) {
      if (!svc?.CompleteOAuthAccountSetup) return 'demo-local'
      return await svc.CompleteOAuthAccountSetup(req)
    },

    async GetOAuthCallbackStatus(req: { state: string }) {
      if (!svc?.GetOAuthCallbackStatus) {
        return { state: req.state, complete: false, accountId: '', error: '', expiresAt: 0 } as OAuthCallbackStatusResponse
      }
      return await svc.GetOAuthCallbackStatus(req)
    },

    async ListProviderPolicies() {
      if (!svc?.ListProviderPolicies) return fallbackPolicies
      return await svc.ListProviderPolicies()
    },

    async GetDiagnosticsHealth() {
      if (!svc?.GetDiagnosticsHealth) {
        return {
          schemaOk: true,
          accountCount: 1,
          messageCount: 0,
          draftCount: 0,
          attachmentCount: 0,
          pendingOAuth: 0,
          providerCounts: { local: 1 },
          checks: [],
          generatedAt: Date.now() / 1000,
        }
      }
      return await svc.GetDiagnosticsHealth()
    },

    async ExportDiagnostics(req: { includeSchema: boolean }) {
      if (!svc?.ExportDiagnostics) {
        return { path: '', sizeBytes: 0, generatedAt: Date.now() / 1000 } as DiagnosticsExportResult
      }
      return await svc.ExportDiagnostics(req)
    },

    async GetUIPreferences() {
      if (!svc?.GetUIPreferences) return { language: 'en', density: 'comfortable' } as UIPreferences
      return await svc.GetUIPreferences()
    },

    async UpdateUIPreferences(req: Partial<UIPreferences>) {
      if (!svc?.UpdateUIPreferences) return { language: req.language ?? 'en', density: req.density ?? 'comfortable' } as UIPreferences
      return await svc.UpdateUIPreferences(req)
    },
  }
}
