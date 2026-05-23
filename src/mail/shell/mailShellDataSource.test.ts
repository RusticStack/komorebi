import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ComposerDraftSaveRequest,
  MailSearchRequest,
  MailSearchResponse,
  MailShellData,
  RecentRecipientSuggestion,
} from "./mailShellState";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

import {
  autosaveComposerDraftFromTauri,
  createAccountFromTauri,
  getAccountAuthStatusFromTauri,
  isTauriRuntime,
  listRecentRecipientsFromTauri,
  loadMailShellDataFromTauri,
  reauthAccountFromTauri,
  removeAccountFromTauri,
  searchMailFromTauri,
  startOAuthAccountReauthFromTauri,
  startOAuthAccountSetupFromTauri,
  testAccountConnectionFromTauri,
  updateAccountSettingsFromTauri,
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
    delete window.__TAURI_INTERNALS__;
  });

  it("detects the Tauri runtime without enabling backend calls in browser-only tests", () => {
    expect(isTauriRuntime()).toBe(false);

    window.__TAURI_INTERNALS__ = {};
    expect(isTauriRuntime()).toBe(true);
  });

  it("maps mail shell reads and search requests to typed Tauri commands", async () => {
    const searchResponse: MailSearchResponse = {
      indexStatus: "ready",
      query: "phoenix",
      results: [],
    };
    invokeMock
      .mockResolvedValueOnce(shellResponse)
      .mockResolvedValueOnce(searchResponse);

    await expect(loadMailShellDataFromTauri()).resolves.toBe(shellResponse);
    expect(invokeMock).toHaveBeenNthCalledWith(1, "load_mail_shell");

    const controller = new AbortController();
    await expect(
      searchMailFromTauri(searchRequest, controller.signal),
    ).resolves.toBe(searchResponse);
    expect(invokeMock).toHaveBeenNthCalledWith(2, "search_mail", {
      request: searchRequest,
    });
  });

  it("never invokes backend search or recipient lookup after caller cancellation", async () => {
    const controller = new AbortController();
    controller.abort();

    await expect(
      searchMailFromTauri(searchRequest, controller.signal),
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
      accountId: "acct_generic",
      credentialsDeleted: true,
      localCacheDeleted: true,
      metadataRemoved: true,
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
      .mockResolvedValueOnce(setupResponse)
      .mockResolvedValueOnce(setupResponse)
      .mockResolvedValueOnce(removalResponse)
      .mockResolvedValueOnce(accountSettingsResponse)
      .mockResolvedValueOnce(connectionResponse)
      .mockResolvedValueOnce(authStatusResponse)
      .mockResolvedValueOnce(setupResponse)
      .mockResolvedValueOnce(setupResponse);

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

    expect(invokeMock).toHaveBeenNthCalledWith(1, "create_account", {
      request: setupRequest,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "reauth_account", {
      request: reauthRequest,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(3, "remove_account", {
      request: removalRequest,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(4, "update_account_settings", {
      request: accountSettingsRequest,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(5, "test_account_connection", {
      request: setupRequest,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(6, "get_account_auth_status", {
      request: authStatusRequest,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(7, "start_oauth_account_setup", {
      request: oauthSetupRequest,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(8, "start_oauth_account_reauth", {
      request: oauthReauthRequest,
    });
  });
});
