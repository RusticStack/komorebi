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
  isTauriRuntime,
  listRecentRecipientsFromTauri,
  loadMailShellDataFromTauri,
  searchMailFromTauri,
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
});
