export type LoadStatus = "ready" | "loading" | "error";

export type AccountStatus =
  | "connected"
  | "syncing"
  | "reconnect-needed"
  | "offline";

export type FolderKind =
  | "inbox"
  | "sent"
  | "drafts"
  | "archive"
  | "trash"
  | "junk"
  | "all"
  | "important"
  | "custom";

export type MailAccount = {
  id: string;
  label: string;
  email: string;
  providerKind?: AccountProviderKind;
  authKind?: AccountAuthKind;
  status: AccountStatus;
  unreadCount: number;
  signatureHtml: string | null;
};

export type MailFolder = {
  id: string;
  accountId: string;
  name: string;
  kind: FolderKind;
  unreadCount: number;
  totalCount: number;
};

export type MailAttachment = {
  id: string;
  displayFilename: string;
  contentType: string;
  sizeBytes: number;
  disposition: "attachment" | "inline";
};

export type MailMessage = {
  id: string;
  accountId: string;
  folderId: string;
  sender: string;
  senderEmail: string;
  subject: string;
  preview: string;
  receivedAt: string;
  unread: boolean;
  flagged: boolean;
  bodyText: string | null;
  bodyHtml: string | null;
  attachments: MailAttachment[];
};

export type MailShellData = {
  accounts: MailAccount[];
  folders: MailFolder[];
  messages: MailMessage[];
};

export type MailSearchRequest = {
  query: string;
  accountId: string | null;
  folderId: string | null;
  limit?: number;
};

export type MailSearchIndexStatus = "ready" | "unavailable";

export type MailSearchResponse = {
  query: string;
  indexStatus: MailSearchIndexStatus;
  results: MailMessage[];
};

export type ComposerAddress = {
  name: string | null;
  address: string;
};

export type ComposerAttachmentDisposition = "attachment" | "inline";

export type ComposerAttachment = {
  id: string;
  accountId: string;
  draftId: string;
  displayFilename: string;
  contentType: string;
  sizeBytes: number;
  disposition: ComposerAttachmentDisposition;
  contentId: string | null;
};

export type ComposerDraftSaveRequest = {
  accountId: string;
  draftId: string;
  subject: string;
  to: ComposerAddress[];
  cc: ComposerAddress[];
  bcc: ComposerAddress[];
  bodyText: string | null;
  bodyHtml: string | null;
  attachmentIds: string[];
};

export type ComposerDraftSaveResponse = {
  id: string;
  accountId: string;
  autosaveRevision: number;
  state: "active" | "discarded" | "sent";
};

export type ComposerDraftDiscardRequest = {
  accountId: string;
  draftId: string;
};

export type ComposerAttachmentStageRequest = {
  accountId: string;
  draftId: string;
  displayFilename: string;
  contentType: string | null;
  disposition: ComposerAttachmentDisposition;
  bytes: number[];
};

export type ComposerSendRequest = ComposerDraftSaveRequest;

export type ComposerSendProgress =
  | "account-resolved"
  | "draft-prepared"
  | "auth-resolved"
  | "attachment-streamed"
  | "mime-composed"
  | "smtp-accepted"
  | "sent-state-recorded"
  | "draft-finalized";

export type ComposerSendResponse = {
  accountId: string;
  draftId: string;
  rfcMessageId: string;
  sentFolderPolicy: string;
  sentFolderSyncState: string;
  providerResponse: string | null;
  messageSizeBytes: number;
  attachmentBytesStreamed: number;
  progress: ComposerSendProgress[];
};

export type RecentRecipientSearchRequest = {
  accountId: string;
  query: string;
  limit?: number;
};

export type RecentRecipientSuggestion = {
  id: string;
  accountId: string;
  name: string | null;
  address: string;
  useCount: number;
  lastUsedAt: string;
};

export type AccountProviderKind = "gmail" | "microsoft" | "generic_imap_smtp";

export type AccountAuthKind = "oauth2" | "app_password" | "manual_password";

export type AccountSecurityMode = "tls" | "starttls" | "none";

export type AccountSetupRequest = {
  accountId?: string | null;
  providerKind: AccountProviderKind;
  displayEmail: string;
  displayName: string | null;
  username: string;
  authKind: AccountAuthKind;
  secret: string;
  imapHost: string | null;
  imapPort: number | null;
  imapSecurity: AccountSecurityMode | null;
  smtpHost: string | null;
  smtpPort: number | null;
  smtpSecurity: AccountSecurityMode | null;
};

export type AccountReauthRequest = {
  accountId: string;
  secret: string;
};

export type OAuthAccountProviderKind = Exclude<
  AccountProviderKind,
  "generic_imap_smtp"
>;

export type OAuthAccountSetupRequest = {
  accountId?: string | null;
  providerKind: OAuthAccountProviderKind;
  displayEmail: string;
  displayName: string | null;
  username: string;
  clientId: string;
  timeoutSeconds?: number | null;
};

export type OAuthAccountReauthRequest = {
  accountId: string;
  clientId: string;
  timeoutSeconds?: number | null;
};

export type AccountRemovalRequest = {
  accountId: string;
  deleteLocalCache: boolean;
};

export type AccountAuthStatusRequest = {
  accountId: string;
};

export type AccountManagementView = {
  id: string;
  providerKind: AccountProviderKind;
  displayEmail: string;
  displayName: string | null;
  username: string;
  authKind: AccountAuthKind;
  authStatus: "connected" | "reconnect_needed" | "disconnected" | "removed";
};

export type AccountSetupResponse = {
  account: AccountManagementView;
  credentialStored: boolean;
};

export type AccountConnectionTestResponse = {
  providerKind: AccountProviderKind;
  authKind: AccountAuthKind;
  status: "configuration-ready" | "connection-ready";
};

export type AccountRemovalResponse = {
  accountId: string;
  credentialsDeleted: boolean;
  localCacheDeleted: boolean;
  metadataRemoved: boolean;
};

export type AccountAuthStatusView = {
  accountId: string;
  providerKind: AccountProviderKind;
  authKind: AccountAuthKind;
  authStatus: "connected" | "reconnect_needed" | "disconnected" | "removed";
  credentialPresent: boolean;
};

export type MailShellLoadState = {
  accounts: LoadStatus;
  folders: LoadStatus;
  messages: LoadStatus;
};

export type MailShellState = {
  data: MailShellData;
  loadState: MailShellLoadState;
  unifiedInbox: boolean;
  selectedAccountId: string | null;
  selectedFolderId: string | null;
  selectedMessageId: string | null;
  searchQuery: string;
};

export const emptyMailShellData: MailShellData = {
  accounts: [],
  folders: [],
  messages: [],
};

export const readyMailShellLoadState: MailShellLoadState = {
  accounts: "ready",
  folders: "ready",
  messages: "ready",
};

export function createMailShellState(
  data: MailShellData = emptyMailShellData,
  loadState: MailShellLoadState = readyMailShellLoadState,
): MailShellState {
  return {
    data,
    loadState,
    unifiedInbox: true,
    selectedAccountId: data.accounts[0]?.id ?? null,
    selectedFolderId: null,
    selectedMessageId: null,
    searchQuery: "",
  };
}

export function loadingMailShellLoadState(): MailShellLoadState {
  return {
    accounts: "loading",
    folders: "loading",
    messages: "loading",
  };
}

export function errorMailShellLoadState(): MailShellLoadState {
  return {
    accounts: "error",
    folders: "error",
    messages: "error",
  };
}

export function foldersForScope(state: MailShellState) {
  if (state.unifiedInbox || !state.selectedAccountId) {
    return state.data.folders;
  }

  return state.data.folders.filter(
    (folder) => folder.accountId === state.selectedAccountId,
  );
}

export function selectUnifiedScope(state: MailShellState): MailShellState {
  return withPreservedSelection({
    ...state,
    unifiedInbox: true,
    selectedAccountId: state.selectedAccountId ?? state.data.accounts[0]?.id ?? null,
    selectedFolderId: null,
  });
}

export function selectAccountScope(
  state: MailShellState,
  selectedAccountId: string,
): MailShellState {
  const selectedFolderBelongsToAccount = state.data.folders.some(
    (folder) =>
      folder.id === state.selectedFolderId &&
      folder.accountId === selectedAccountId,
  );

  return withPreservedSelection({
    ...state,
    unifiedInbox: false,
    selectedAccountId,
    selectedFolderId: selectedFolderBelongsToAccount
      ? state.selectedFolderId
      : null,
  });
}

export function selectFolderScope(
  state: MailShellState,
  selectedFolderId: string | null,
): MailShellState {
  return withPreservedSelection({
    ...state,
    selectedFolderId,
  });
}

export function mergeMailShellData(
  state: MailShellState,
  data: MailShellData,
  loadState: MailShellLoadState = readyMailShellLoadState,
): MailShellState {
  const selectedAccountExists = data.accounts.some(
    (account) => account.id === state.selectedAccountId,
  );
  const selectedFolderExists = data.folders.some(
    (folder) => folder.id === state.selectedFolderId,
  );

  return withPreservedSelection({
    ...state,
    data,
    loadState,
    selectedAccountId: selectedAccountExists
      ? state.selectedAccountId
      : data.accounts[0]?.id ?? null,
    selectedFolderId: selectedFolderExists ? state.selectedFolderId : null,
  });
}

export function messageIsVisibleInScope(
  state: MailShellState,
  messageId: string,
) {
  return messagesForScope(state).some((message) => message.id === messageId);
}

function withPreservedSelection(state: MailShellState): MailShellState {
  if (
    state.selectedMessageId &&
    !messageIsVisibleInScope(state, state.selectedMessageId)
  ) {
    return {
      ...state,
      selectedMessageId: null,
    };
  }

  return state;
}

export function messagesForScope(state: MailShellState) {
  const query = state.searchQuery.trim().toLocaleLowerCase();
  const scopedFolders = new Map(
    foldersForScope(state).map((folder) => [folder.id, folder]),
  );

  return state.data.messages.filter((message) => {
    const folder = scopedFolders.get(message.folderId);
    if (!folder) {
      return false;
    }

    if (state.selectedFolderId) {
      if (message.folderId !== state.selectedFolderId) {
        return false;
      }
    } else if (state.unifiedInbox && !query && folder.kind !== "inbox") {
      return false;
    }

    if (!state.unifiedInbox && state.selectedAccountId) {
      if (message.accountId !== state.selectedAccountId) {
        return false;
      }
    }

    if (!query) {
      return true;
    }

    return [
      message.sender,
      message.senderEmail,
      message.subject,
      message.preview,
      message.bodyText ?? "",
    ].some((value) => value.toLocaleLowerCase().includes(query));
  });
}

export function selectedMessage(state: MailShellState) {
  if (!state.selectedMessageId) {
    return null;
  }

  return (
    state.data.messages.find((message) => message.id === state.selectedMessageId) ??
    null
  );
}

export function selectedAccount(state: MailShellState) {
  if (!state.selectedAccountId) {
    return null;
  }

  return (
    state.data.accounts.find(
      (account) => account.id === state.selectedAccountId,
    ) ?? null
  );
}

export function selectedFolder(state: MailShellState) {
  if (!state.selectedFolderId) {
    return null;
  }

  return (
    state.data.folders.find((folder) => folder.id === state.selectedFolderId) ??
    null
  );
}
