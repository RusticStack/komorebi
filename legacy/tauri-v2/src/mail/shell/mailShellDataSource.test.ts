import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  AttachmentCommandRequest,
  AttachmentDownloadResponse,
  AttachmentPreviewResponse,
  AttachmentSaveResponse,
  ComposerForwardedAttachmentStageRequest,
  ComposerDraftSaveRequest,
  DiagnosticsExportRequest,
  DiagnosticsExportResponse,
  DiagnosticsHealthView,
  ExternalLinkOpenRequest,
  ExternalLinkOpenResponse,
  MailActionRequest,
  MailActionResponse,
  MailMessagePageRequest,
  MailMessagePageResponse,
  MailSearchRequest,
  MailSearchResponse,
  MailShellData,
  RecentRecipientSuggestion,
  RemoteContentPreferenceRequest,
  RemoteContentPreferenceResponse,
} from "./mailShellState";

const invokeMock = vi.hoisted(() => vi.fn());
const listenMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

import {
  allowRemoteContentFromTauri,
  autosaveComposerDraftFromTauri,
  createAccountFromTauri,
  discoverAccountSettingsFromTauri,
  downloadAttachmentFromTauri,
  exportDiagnosticsBundleFromTauri,
  getAccountAuthStatusFromTauri,
  getDiagnosticsHealthFromTauri,
  getUiPreferencesFromTauri,
  isTauriRuntime,
  listComposerDraftsFromTauri,
  listRecentRecipientsFromTauri,
  loadMessagePageFromTauri,
  loadMailShellDataFromTauri,
  openExternalLinkFromTauri,
  performMailActionFromTauri,
  previewAttachmentFromTauri,
  reauthAccountFromTauri,
  removeAccountFromTauri,
  saveAttachmentFromTauri,
  searchMailFromTauri,
  stageForwardedAttachmentsFromTauri,
  startOAuthAccountReauthFromTauri,
  startOAuthAccountSetupFromTauri,
  subscribeToSyncStatusFromTauri,
  SYNC_STATUS_EVENT_NAME,
  testAccountConnectionFromTauri,
  updateAccountSettingsFromTauri,
  updateUiPreferencesFromTauri,
} from "./mailShellDataSource";

const shellResponse: MailShellData = {
  accounts: [],
  folders: [],
  messages: [],
};

const searchRequest: MailSearchRequest = {
  accountId: "acct_personal",
  folderId: "folder_inbox",
  limit: 100,
  query: "phoenix",
};

describe("mailShellDataSource", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockReset();
    delete window.__TAURI_INTERNALS__;
  });

  it("detects the Tauri runtime without enabling backend calls in browser-only tests", () => {
    expect(isTauriRuntime()).toBe(false);

    window.__TAURI_INTERNALS__ = {};
    expect(isTauriRuntime()).toBe(true);
  });

  it("maps mail shell reads and search requests to typed Tauri commands", async () => {
    const pageRequest: MailMessagePageRequest = {
      accountId: "acct_personal",
      cursor: null,
      folderId: "folder_inbox",
      limit: 100,
      unifiedInbox: false,
    };
    const pageResponse: MailMessagePageResponse = {
      hasMore: true,
      messages: [],
      nextCursor: {
        messageId: "msg_099",
        sortKey: "2026-05-22T09:00:00Z",
      },
      totalCount: 250,
    };
    const searchResponse: MailSearchResponse = {
      indexStatus: "ready",
      query: "phoenix",
      results: [],
    };
    invokeMock
      .mockResolvedValueOnce(shellResponse)
      .mockResolvedValueOnce(pageResponse)
      .mockResolvedValueOnce(searchResponse);

    await expect(loadMailShellDataFromTauri()).resolves.toBe(shellResponse);
    expect(invokeMock).toHaveBeenNthCalledWith(1, "load_mail_shell");

    const controller = new AbortController();
    await expect(
      loadMessagePageFromTauri(pageRequest, controller.signal),
    ).resolves.toBe(pageResponse);
    expect(invokeMock).toHaveBeenNthCalledWith(2, "load_message_page", {
      request: pageRequest,
    });

    await expect(
      searchMailFromTauri(searchRequest, controller.signal),
    ).resolves.toBe(searchResponse);
    expect(invokeMock).toHaveBeenNthCalledWith(3, "search_mail", {
      request: searchRequest,
    });
  });

  it("maps attachment preview, save, and download to typed Tauri commands", async () => {
    const request: AttachmentCommandRequest = {
      accountId: "acct_personal",
      folderId: "folder_inbox",
      messageId: "msg_invoice",
      attachmentId: "att_invoice",
      userRequested: true,
    };
    const previewResponse: AttachmentPreviewResponse = {
      attachmentId: "att_invoice",
      autoOpen: false,
      bytes: null,
      contentType: "text/plain",
      displayFilename: "invoice.txt",
      kind: "plain-text",
      sizeBytes: 13,
      text: "Invoice notes",
    };
    const saveResponse: AttachmentSaveResponse = {
      attachmentId: "att_invoice",
      bytesWritten: 13,
      contentType: "text/plain",
      destinationLabel: "invoice.txt",
      displayFilename: "invoice.txt",
      savedFilename: "invoice.txt",
      sizeBytes: 13,
    };
    const downloadResponse: AttachmentDownloadResponse = {
      attachmentId: "att_invoice",
      bytes: [73, 110, 118, 111, 105, 99, 101],
      contentType: "text/plain",
      displayFilename: "invoice.txt",
      sizeBytes: 7,
    };
    invokeMock
      .mockResolvedValueOnce(previewResponse)
      .mockResolvedValueOnce(saveResponse)
      .mockResolvedValueOnce(downloadResponse);

    await expect(previewAttachmentFromTauri(request)).resolves.toBe(
      previewResponse,
    );
    await expect(saveAttachmentFromTauri(request)).resolves.toBe(saveResponse);
    await expect(downloadAttachmentFromTauri(request)).resolves.toBe(
      downloadResponse,
    );

    expect(invokeMock).toHaveBeenNthCalledWith(1, "preview_attachment", {
      request,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "save_attachment", {
      request,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(3, "download_attachment", {
      request,
    });
  });

  it("maps remote-content preferences to the typed Tauri command", async () => {
    const request: RemoteContentPreferenceRequest = {
      accountId: "acct_work",
      folderId: "folder_inbox",
      messageId: "msg_invoice",
      scope: "sender",
      userRequested: true,
    };
    const response: RemoteContentPreferenceResponse = {
      accountId: "acct_work",
      messageId: "msg_invoice",
      remoteContentAllowed: true,
      scope: "sender",
      senderEmail: "finance@example.com",
    };
    invokeMock.mockResolvedValueOnce(response);

    await expect(allowRemoteContentFromTauri(request)).resolves.toBe(response);
    expect(invokeMock).toHaveBeenCalledWith("allow_remote_content", {
      request,
    });
  });

  it("maps UI preference reads and writes to typed Tauri commands", async () => {
    invokeMock
      .mockResolvedValueOnce({ language: null })
      .mockResolvedValueOnce({ language: "pt-PT" });

    await expect(getUiPreferencesFromTauri()).resolves.toEqual({
      language: null,
    });
    await expect(
      updateUiPreferencesFromTauri({ language: "pt-PT" }),
    ).resolves.toEqual({
      language: "pt-PT",
    });

    expect(invokeMock).toHaveBeenNthCalledWith(1, "get_ui_preferences");
    expect(invokeMock).toHaveBeenNthCalledWith(2, "update_ui_preferences", {
      request: { language: "pt-PT" },
    });
  });

  it("maps diagnostics export to the typed Tauri command", async () => {
    const request: DiagnosticsExportRequest = {
      userRequested: true,
      includeSyntheticFixtures: false,
    };
    const response: DiagnosticsExportResponse = {
      bytesWritten: 4096,
      destinationLabel: "komorebi-diagnostics.json",
      includedLogEntries: 4,
      includedSyntheticFixtures: false,
    };
    invokeMock.mockResolvedValueOnce(response);

    await expect(exportDiagnosticsBundleFromTauri(request)).resolves.toBe(
      response,
    );
    expect(invokeMock).toHaveBeenCalledWith("export_diagnostics_bundle", {
      request,
    });
  });

  it("maps diagnostics health to an inputless typed Tauri command", async () => {
    const response: DiagnosticsHealthView = {
      attachments: {
        checkedFiles: 7,
        hashMismatches: 0,
        invalidPaths: 0,
        metadataCount: 7,
        missingFiles: 0,
        oversizedFiles: 0,
        sizeMismatches: 0,
        storedCount: 7,
        unreadableFiles: 0,
      },
      cache: {
        accountCount: 1,
        attachmentMetadataCount: 7,
        authStatusCounts: { connected: 1 },
        folderCount: 8,
        latestSchemaVersion: 10,
        mailCachePresent: true,
        messageCount: 40,
        migrationStatus: "current",
        schemaVersion: 10,
        settingsCount: 4,
        syncStatusCounts: { idle: 1 },
      },
      generatedAtUnixMs: 1_779_716_800_000,
      remediationSteps: ["healthy"],
      status: "healthy",
      sync: {
        categoryCounts: {},
        errorCount: 0,
        lastRetryAtUnixMs: null,
        logPresent: true,
        recentEvents: [],
        retryEventCount: 0,
        warningCount: 0,
      },
    };
    invokeMock.mockResolvedValueOnce(response);

    await expect(getDiagnosticsHealthFromTauri()).resolves.toBe(response);
    expect(invokeMock).toHaveBeenCalledWith("get_diagnostics_health");
  });

  it("maps external link opening to the typed Tauri command", async () => {
    const request: ExternalLinkOpenRequest = {
      url: "mailto:person@example.com?subject=Hello",
      userRequested: true,
    };
    const response: ExternalLinkOpenResponse = {
      opened: true,
      openedUrl: request.url,
    };
    invokeMock.mockResolvedValueOnce(response);

    await expect(openExternalLinkFromTauri(request)).resolves.toBe(response);
    expect(invokeMock).toHaveBeenCalledWith("open_external_link", {
      request,
    });
  });

  it("maps mail actions to the typed Tauri command", async () => {
    const request: MailActionRequest = {
      accountId: "acct_personal",
      action: { kind: "mark-read", read: true },
      confirmed: false,
      folderId: "folder_inbox",
      messageId: "msg_invoice",
      userRequested: true,
    };
    const response: MailActionResponse = {
      accountId: "acct_personal",
      action: request.action,
      mutation: { kind: "flags-updated", read: true },
      retry: { kind: "retryable", maxAttempts: 3 },
      undo: {
        accountId: "acct_personal",
        action: { kind: "mark-read", read: false },
        folderId: "folder_inbox",
        messageId: "msg_invoice",
      },
    };
    invokeMock.mockResolvedValueOnce(response);

    await expect(performMailActionFromTauri(request)).resolves.toBe(response);
    expect(invokeMock).toHaveBeenCalledWith("perform_mail_action", {
      request,
    });
  });

  it("never invokes backend search or recipient lookup after caller cancellation", async () => {
    const controller = new AbortController();
    controller.abort();

    await expect(
      searchMailFromTauri(searchRequest, controller.signal),
    ).rejects.toMatchObject({ name: "AbortError" });
    await expect(
      loadMessagePageFromTauri(
        {
          accountId: "acct_personal",
          cursor: null,
          folderId: "folder_inbox",
          limit: 100,
          unifiedInbox: false,
        },
        controller.signal,
      ),
    ).rejects.toMatchObject({ name: "AbortError" });
    await expect(
      listRecentRecipientsFromTauri(
        { accountId: "acct_personal", query: "ada", limit: 10 },
        controller.signal,
      ),
    ).rejects.toMatchObject({ name: "AbortError" });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("drops backend search and autocomplete responses that arrive after cancellation", async () => {
    const searchController = new AbortController();
    const recipientController = new AbortController();
    const recipientResponse: RecentRecipientSuggestion[] = [
      {
        accountId: "acct_personal",
        address: "ada@example.com",
        id: "rr_ada",
        lastUsedAt: "2026-05-23T10:00:00Z",
        name: "Ada",
        useCount: 2,
      },
    ];
    invokeMock
      .mockImplementationOnce(async () => {
        searchController.abort();
        return {
          indexStatus: "ready",
          query: "phoenix",
          results: [],
        } satisfies MailSearchResponse;
      })
      .mockImplementationOnce(async () => {
        recipientController.abort();
        return recipientResponse;
      });

    await expect(
      searchMailFromTauri(searchRequest, searchController.signal),
    ).rejects.toMatchObject({ name: "AbortError" });
    await expect(
      listRecentRecipientsFromTauri(
        { accountId: "acct_personal", query: "ada", limit: 10 },
        recipientController.signal,
      ),
    ).rejects.toMatchObject({ name: "AbortError" });

    expect(invokeMock).toHaveBeenNthCalledWith(1, "search_mail", {
      request: searchRequest,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "list_recent_recipients", {
      request: { accountId: "acct_personal", query: "ada", limit: 10 },
    });
  });

  it("keeps composer autosave behind the typed Rust command payload", async () => {
    const request: ComposerDraftSaveRequest = {
      accountId: "acct_personal",
      attachmentIds: ["catt_report"],
      bcc: [],
      bodyHtml: "<p>Safe composer body</p>",
      bodyText: "Safe composer body",
      cc: [],
      draftId: "draft_123",
      inReplyTo: "<source@example.com>",
      references: ["<root@example.com>", "<source@example.com>"],
      subject: "Quarterly review",
      to: [{ address: "ada@example.com", name: "Ada" }],
    };
    const response = {
      accountId: "acct_personal",
      autosaveRevision: 7,
      id: "draft_123",
      state: "active" as const,
    };
    invokeMock.mockResolvedValueOnce(response);

    await expect(autosaveComposerDraftFromTauri(request)).resolves.toBe(
      response,
    );
    expect(invokeMock).toHaveBeenCalledWith("autosave_composer_draft", {
      request,
    });
  });

  it("loads active composer drafts through the reviewed read command", async () => {
    const drafts = [
      {
        accountId: "acct_personal",
        attachments: [],
        autosaveRevision: 3,
        bcc: [],
        bodyHtml: "<p>Draft body</p>",
        bodyText: "Draft body",
        cc: [],
        id: "draft_123",
        inReplyTo: "<source@example.com>",
        references: ["<source@example.com>"],
        subject: "Saved local draft",
        to: [{ address: "ada@example.com", name: "Ada" }],
      },
    ];
    invokeMock.mockResolvedValueOnce(drafts);

    await expect(listComposerDraftsFromTauri()).resolves.toBe(drafts);
    expect(invokeMock).toHaveBeenCalledWith("list_composer_drafts");
  });

  it("stages forwarded attachments through the reviewed backend command", async () => {
    const request: ComposerForwardedAttachmentStageRequest = {
      accountId: "acct_personal",
      draftId: "draft_reply",
      sourceAttachmentIds: ["att_invoice"],
      sourceFolderId: "folder_inbox",
      sourceMessageId: "msg_invoice",
      userRequested: true,
    };
    const response = [
      {
        accountId: "acct_personal",
        contentId: null,
        contentType: "application/pdf",
        displayFilename: "invoice.pdf",
        disposition: "attachment" as const,
        draftId: "draft_reply",
        id: "catt_invoice",
        sizeBytes: 2048,
      },
    ];
    invokeMock.mockResolvedValueOnce(response);

    await expect(stageForwardedAttachmentsFromTauri(request)).resolves.toBe(
      response,
    );
    expect(invokeMock).toHaveBeenCalledWith("stage_forwarded_attachments", {
      request,
    });
  });

  it("maps account management workflows to typed Tauri commands", async () => {
    const setupRequest = {
      authKind: "app_password" as const,
      displayEmail: "person@example.com",
      displayName: "Personal",
      imapHost: "imap.example.com",
      imapPort: 993,
      imapSecurity: "tls" as const,
      providerKind: "generic_imap_smtp" as const,
      secret: "app-password",
      smtpHost: "smtp.example.com",
      smtpPort: 587,
      smtpSecurity: "starttls" as const,
      username: "person@example.com",
    };
    const setupResponse = {
      account: {
        authKind: "app_password" as const,
        authStatus: "connected" as const,
        displayEmail: "person@example.com",
        displayName: "Personal",
        id: "acct_generic",
        providerKind: "generic_imap_smtp" as const,
        username: "person@example.com",
      },
      credentialStored: true,
    };
    const discoveryRequest = { displayEmail: "person@fastmail.com" };
    const discoveryResponse = {
      authKind: "app_password" as const,
      confidence: "high" as const,
      imapHost: "imap.fastmail.com",
      imapPort: 993,
      imapSecurity: "tls" as const,
      providerKind: "generic_imap_smtp" as const,
      smtpHost: "smtp.fastmail.com",
      smtpPort: 465,
      smtpSecurity: "tls" as const,
      source: "known-provider" as const,
      username: "person@fastmail.com",
    };
    const reauthRequest = {
      accountId: "acct_generic",
      secret: "replacement",
    };
    const oauthSetupRequest = {
      clientId: "desktop-client-id",
      displayEmail: "person@gmail.com",
      displayName: null,
      providerKind: "gmail" as const,
      username: "person@gmail.com",
    };
    const oauthReauthRequest = {
      accountId: "acct_gmail",
      clientId: "desktop-client-id",
    };
    const removalRequest = {
      accountId: "acct_generic",
      deleteLocalCache: true,
    };
    const accountSettingsRequest = {
      accountId: "acct_generic",
      displayName: "Personal",
      localPreferences: { includeInUnifiedInbox: false },
      signatureHtml: "<p>Regards</p>",
    };
    const accountSettingsResponse = {
      account: setupResponse.account,
      localPreferences: { includeInUnifiedInbox: false },
      signatureHtml: "<p>Regards</p>",
    };
    const removalResponse = {
      accessCacheCleared: true,
      accountId: "acct_generic",
      credentialsDeleted: true,
      localCacheDeleted: true,
      localCacheRetained: false,
      metadataRemoved: true,
      providerRevocation: "not-applicable" as const,
      syncStopped: true,
    };
    const authStatusRequest = { accountId: "acct_generic" };
    const authStatusResponse = {
      accountId: "acct_generic",
      authKind: "app_password" as const,
      authStatus: "connected" as const,
      credentialPresent: true,
      providerKind: "generic_imap_smtp" as const,
    };
    const connectionResponse = {
      authKind: "app_password" as const,
      providerKind: "generic_imap_smtp" as const,
      status: "configuration-ready" as const,
    };
    invokeMock
      .mockResolvedValueOnce(discoveryResponse)
      .mockResolvedValueOnce(setupResponse)
      .mockResolvedValueOnce(setupResponse)
      .mockResolvedValueOnce(removalResponse)
      .mockResolvedValueOnce(accountSettingsResponse)
      .mockResolvedValueOnce(connectionResponse)
      .mockResolvedValueOnce(authStatusResponse)
      .mockResolvedValueOnce(setupResponse)
      .mockResolvedValueOnce(setupResponse);

    await expect(
      discoverAccountSettingsFromTauri(discoveryRequest),
    ).resolves.toBe(discoveryResponse);
    await expect(createAccountFromTauri(setupRequest)).resolves.toBe(
      setupResponse,
    );
    await expect(reauthAccountFromTauri(reauthRequest)).resolves.toBe(
      setupResponse,
    );
    await expect(removeAccountFromTauri(removalRequest)).resolves.toBe(
      removalResponse,
    );
    await expect(
      updateAccountSettingsFromTauri(accountSettingsRequest),
    ).resolves.toBe(accountSettingsResponse);
    await expect(testAccountConnectionFromTauri(setupRequest)).resolves.toBe(
      connectionResponse,
    );
    await expect(
      getAccountAuthStatusFromTauri(authStatusRequest),
    ).resolves.toBe(authStatusResponse);
    await expect(
      startOAuthAccountSetupFromTauri(oauthSetupRequest),
    ).resolves.toBe(setupResponse);
    await expect(
      startOAuthAccountReauthFromTauri(oauthReauthRequest),
    ).resolves.toBe(setupResponse);

    expect(invokeMock).toHaveBeenNthCalledWith(1, "discover_account_settings", {
      request: discoveryRequest,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "create_account", {
      request: setupRequest,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(3, "reauth_account", {
      request: reauthRequest,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(4, "remove_account", {
      request: removalRequest,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(5, "update_account_settings", {
      request: accountSettingsRequest,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(6, "test_account_connection", {
      request: setupRequest,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(7, "get_account_auth_status", {
      request: authStatusRequest,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(8, "start_oauth_account_setup", {
      request: oauthSetupRequest,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(9, "start_oauth_account_reauth", {
      request: oauthReauthRequest,
    });
  });

  it("subscribes to backend sync status events without exposing emit permissions", async () => {
    const unlisten = vi.fn();
    const eventHandlers: Array<(event: { payload: unknown }) => void> = [];
    listenMock.mockImplementationOnce(
      (_eventName: string, handler: (event: { payload: unknown }) => void) => {
        eventHandlers.push(handler);
        return Promise.resolve(unlisten);
      },
    );
    const handler = vi.fn();

    await expect(subscribeToSyncStatusFromTauri(handler)).resolves.toBe(
      unlisten,
    );
    expect(listenMock).toHaveBeenCalledWith(
      SYNC_STATUS_EVENT_NAME,
      expect.any(Function),
    );

    const eventHandler = eventHandlers[0];
    if (!eventHandler) {
      throw new Error("sync status event handler was not registered");
    }
    eventHandler({
      payload: {
        accountId: "acct_personal",
        status: {
          action: "reauthenticate",
          code: "auth_reconnect_required",
          message: "Reauthentication is required for this account.",
          state: "error",
          updatedAt: 1779552060,
        },
      },
    });

    expect(handler).toHaveBeenCalledWith({
      accountId: "acct_personal",
      status: {
        action: "reauthenticate",
        code: "auth_reconnect_required",
        message: "Reauthentication is required for this account.",
        state: "error",
        updatedAt: 1779552060,
      },
    });
  });
});
