import { describe, expect, it } from "vitest";
import {
  createMailShellState,
  mergeMailShellData,
  messagesForScope,
  selectAccountScope,
  selectFolderScope,
  selectUnifiedScope,
  type MailShellData,
} from "./mailShellState";

const stateFixture: MailShellData = {
  accounts: [
    {
      email: "personal@example.com",
      id: "acct_personal",
      label: "Personal",
      signatureHtml: null,
      status: "connected",
      unreadCount: 2,
    },
    {
      email: "work@example.com",
      id: "acct_work",
      label: "Work",
      signatureHtml: null,
      status: "syncing",
      unreadCount: 1,
    },
  ],
  folders: [
    {
      accountId: "acct_personal",
      id: "folder_personal_inbox",
      kind: "inbox",
      name: "Inbox",
      totalCount: 1,
      unreadCount: 1,
    },
    {
      accountId: "acct_personal",
      id: "folder_personal_archive",
      kind: "archive",
      name: "Archive",
      totalCount: 1,
      unreadCount: 0,
    },
    {
      accountId: "acct_work",
      id: "folder_work_inbox",
      kind: "inbox",
      name: "Inbox",
      totalCount: 1,
      unreadCount: 1,
    },
  ],
  messages: [
    {
      accountId: "acct_personal",
      attachments: [],
      bodyHtml: null,
      bodyText: "Local cache roadmap body",
      flagged: false,
      folderId: "folder_personal_inbox",
      id: "msg_personal_inbox",
      preview: "Roadmap preview",
      receivedAt: "2026-05-23T09:00:00",
      sender: "Ada",
      senderEmail: "ada@example.com",
      subject: "Roadmap review",
      unread: true,
    },
    {
      accountId: "acct_personal",
      attachments: [],
      bodyHtml: null,
      bodyText: "Archived receipt body",
      flagged: false,
      folderId: "folder_personal_archive",
      id: "msg_personal_archive",
      preview: "Archived preview",
      receivedAt: "2026-05-22T09:00:00",
      sender: "Archive",
      senderEmail: "archive@example.com",
      subject: "Archived receipt",
      unread: false,
    },
    {
      accountId: "acct_work",
      attachments: [],
      bodyHtml: null,
      bodyText: "Invoice body",
      flagged: true,
      folderId: "folder_work_inbox",
      id: "msg_work_inbox",
      preview: "Invoice preview",
      receivedAt: "2026-05-23T08:00:00",
      sender: "Finance",
      senderEmail: "finance@example.com",
      subject: "Invoice question",
      unread: true,
    },
  ],
};

describe("mailShellState", () => {
  it("keeps unified, account, folder, and local-search message scopes deterministic", () => {
    const unified = createMailShellState(stateFixture);
    expect(messagesForScope(unified).map((message) => message.id)).toEqual([
      "msg_personal_inbox",
      "msg_work_inbox",
    ]);

    const personal = selectAccountScope(unified, "acct_personal");
    expect(messagesForScope(personal).map((message) => message.id)).toEqual([
      "msg_personal_inbox",
      "msg_personal_archive",
    ]);

    const archived = selectFolderScope(personal, "folder_personal_archive");
    expect(messagesForScope(archived).map((message) => message.id)).toEqual([
      "msg_personal_archive",
    ]);

    const searched = {
      ...selectUnifiedScope(unified),
      searchQuery: "receipt",
    };
    expect(messagesForScope(searched).map((message) => message.id)).toEqual([
      "msg_personal_archive",
    ]);
  });

  it("preserves selected messages across compatible cache reloads and clears stale backend rows", () => {
    const selected = {
      ...selectFolderScope(
        selectAccountScope(createMailShellState(stateFixture), "acct_personal"),
        "folder_personal_inbox",
      ),
      selectedMessageId: "msg_personal_inbox",
    };

    const refreshed = mergeMailShellData(selected, {
      ...stateFixture,
      messages: stateFixture.messages.map((message) =>
        message.id === "msg_personal_inbox"
          ? { ...message, preview: "Reloaded backend preview" }
          : message,
      ),
    });
    expect(refreshed.selectedAccountId).toBe("acct_personal");
    expect(refreshed.selectedFolderId).toBe("folder_personal_inbox");
    expect(refreshed.selectedMessageId).toBe("msg_personal_inbox");
    expect(
      messagesForScope(refreshed).find(
        (message) => message.id === "msg_personal_inbox",
      )?.preview,
    ).toBe("Reloaded backend preview");

    const staleReload = mergeMailShellData(selected, {
      ...stateFixture,
      folders: stateFixture.folders.filter(
        (folder) => folder.id !== "folder_personal_inbox",
      ),
      messages: stateFixture.messages.filter(
        (message) => message.id !== "msg_personal_inbox",
      ),
    });
    expect(staleReload.selectedAccountId).toBe("acct_personal");
    expect(staleReload.selectedFolderId).toBeNull();
    expect(staleReload.selectedMessageId).toBeNull();
  });
});
