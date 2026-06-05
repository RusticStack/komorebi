// Real Wails v3 binding bridge for the SolidJS frontend.
// All calls go to the Go mail.Service (backed by real modernc/sqlite).

declare global {
  interface Window {
    go?: {
      mail?: {
        Service?: {
          LoadMailShell: () => Promise<any>
          LoadMessagePage: (folderID: string, cursor: number, limit: number) => Promise<any>
          MarkRead: (messageID: string, read: boolean) => Promise<void>
          SearchMail: (query: string, limit: number) => Promise<any>
          ListDrafts: () => Promise<any>
          SaveDraft: (draft: any) => Promise<void>
          DeleteDraft: (id: string) => Promise<void>
          SendDraft: (id: string) => Promise<void>
          AddAccount: (req: {
            email: string
            displayName: string
            providerKind: string
            password?: string
            imapHost?: string
            imapPort?: number
            imapSecurity?: string
            smtpHost?: string
            smtpPort?: number
            smtpSecurity?: string
          }) => Promise<string>
          ListAccounts: () => Promise<any>
          TestAndSyncIMAP: (accountID: string, host: string, port: number, username: string, password: string, useTLS: boolean) => Promise<void>
          SyncAccount: (accountID: string) => Promise<void>
          RemoveAccount: (accountID: string, deleteCache: boolean) => Promise<void>
          // Future real methods (composer send, accounts, etc.) will be added here
        }
      }
    }
  }
}

export function useWails() {
  const svc = window.go?.mail?.Service

  return {
    async LoadMailShell() {
      if (!svc?.LoadMailShell) {
        console.warn('[wails] Bindings not ready - using dev fallback with real schema shape')
        // This fallback shape matches the real DB seeder so the UI works even before wails3 dev injects bindings
        return {
          accounts: [{ id: 'demo-local', email: 'you@local.test', displayName: 'Local Demo', provider: 'local' }],
          folders: [
            { id: 'inbox', accountId: 'demo-local', name: 'Inbox', displayName: 'Inbox', specialUse: 'inbox', unreadCount: 14 },
            { id: 'sent', accountId: 'demo-local', name: 'Sent', displayName: 'Sent', specialUse: 'sent', unreadCount: 0 },
            { id: 'drafts', accountId: 'demo-local', name: 'Drafts', displayName: 'Drafts', specialUse: 'drafts', unreadCount: 0 },
          ],
          messages: [],
          hasMore: true,
          total: 48,
        }
      }
      return await svc.LoadMailShell()
    },

    async LoadMessagePage(folderID: string, cursor: number, limit: number) {
      if (!svc?.LoadMessagePage) return { messages: [], nextCursor: 0, hasMore: false }
      return await svc.LoadMessagePage(folderID, cursor, limit)
    },

    async SearchMail(query: string, limit: number) {
      if (!svc?.SearchMail) return []
      return await svc.SearchMail(query, limit)
    },

    async MarkRead(id: string, read: boolean) {
      if (!svc?.MarkRead) return
      await svc.MarkRead(id, read)
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

    async ListAccounts() {
      if (!svc?.ListAccounts) return []
      return await svc.ListAccounts()
    },

    async TestAndSyncIMAP(accountID: string, host: string, port: number, username: string, password: string, useTLS: boolean) {
      if (!svc?.TestAndSyncIMAP) return
      await svc.TestAndSyncIMAP(accountID, host, port, username, password, useTLS)
    },

    async SyncAccount(accountID: string) {
      if (!svc?.SyncAccount) return
      await svc.SyncAccount(accountID)
    },

    async RemoveAccount(accountID: string, deleteCache: boolean) {
      if (!svc?.RemoveAccount) return
      await svc.RemoveAccount(accountID, deleteCache)
    },
  }
}
