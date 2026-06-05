export interface AccountView {
  id: string
  email: string
  displayName: string
  provider: string

  // Per-account real server settings (stored in DB)
  imapHost: string
  imapPort: number
  imapSecurity: string

  smtpHost: string
  smtpPort: number
  smtpSecurity: string
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
