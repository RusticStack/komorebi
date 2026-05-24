import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { vi } from "vitest";
import App from "./App";
import { formatDateTime } from "./i18n";
import type {
  AttachmentCommandRequest,
  AttachmentDownloadResponse,
  AttachmentPreviewResponse,
  AttachmentSaveResponse,
  ComposerAttachment,
  ComposerAttachmentStageRequest,
  ComposerDraft,
  ComposerDraftDiscardRequest,
  ComposerDraftSaveRequest,
  ComposerDraftSaveResponse,
  ComposerSendRequest,
  ComposerSendResponse,
  ExternalLinkOpenRequest,
  ExternalLinkOpenResponse,
  MailActionRequest,
  MailActionResponse,
  MailMessage,
  MailSearchRequest,
  MailSearchResponse,
  MailShellData,
  MailShellLoadState,
  RecentRecipientSearchRequest,
  RecentRecipientSuggestion,
  RemoteContentPreferenceRequest,
  RemoteContentPreferenceResponse,
} from "./mail/shell/mailShellState";

const fixtureMail: MailShellData = {
  accounts: [
    {
      id: "acct_personal",
      label: "Personal",
      email: "person@example.com",
      status: "connected",
      unreadCount: 2,
      signatureHtml: "<p>Personal signature</p>",
    },
    {
      id: "acct_work",
      label: "Work",
      email: "work@example.com",
      status: "syncing",
      unreadCount: 1,
      signatureHtml: null,
    },
  ],
  folders: [
    {
      id: "folder_personal_inbox",
      accountId: "acct_personal",
      name: "Inbox",
      kind: "inbox",
      unreadCount: 1,
      totalCount: 2,
    },
    {
      id: "folder_personal_archive",
      accountId: "acct_personal",
      name: "Archive",
      kind: "archive",
      unreadCount: 0,
      totalCount: 1,
    },
    {
      id: "folder_personal_trash",
      accountId: "acct_personal",
      name: "Trash",
      kind: "trash",
      unreadCount: 0,
      totalCount: 0,
    },
    {
      id: "folder_work_inbox",
      accountId: "acct_work",
      name: "Inbox",
      kind: "inbox",
      unreadCount: 1,
      totalCount: 1,
    },
    {
      id: "folder_work_archive",
      accountId: "acct_work",
      name: "Archive",
      kind: "archive",
      unreadCount: 0,
      totalCount: 0,
    },
    {
      id: "folder_work_trash",
      accountId: "acct_work",
      name: "Trash",
      kind: "trash",
      unreadCount: 0,
      totalCount: 1,
    },
  ],
  messages: [
    {
      id: "msg_roadmap",
      accountId: "acct_personal",
      folderId: "folder_personal_inbox",
      sender: "Ada",
      senderEmail: "ada@example.com",
      subject: "Roadmap review",
      preview: "Review notes for the local cache work.",
      receivedAt: "2026-05-22T09:30:00",
      unread: true,
      flagged: false,
      bodyText: "Review notes for the local cache work.",
      bodyHtml: null,
      attachments: [],
    },
    {
      id: "msg_archive",
      accountId: "acct_personal",
      folderId: "folder_personal_archive",
      sender: "Archive Bot",
      senderEmail: "archive@example.com",
      subject: "Archived receipt",
      preview: "Stored outside the unified inbox.",
      receivedAt: "2026-05-21T09:30:00",
      unread: false,
      flagged: false,
      bodyText: "Stored outside the unified inbox.",
      bodyHtml: null,
      attachments: [],
    },
    {
      id: "msg_invoice",
      accountId: "acct_work",
      folderId: "folder_work_inbox",
      sender: "Finance",
      senderEmail: "finance@example.com",
      subject: "Invoice question",
      preview: "Can you confirm the invoice number?",
      receivedAt: "2026-05-22T08:15:00",
      unread: true,
      flagged: true,
      bodyText: "Can you confirm the invoice number?",
      bodyHtml:
        '<p>Can you <strong>confirm</strong> the invoice number in the <a href="https://example.com/invoice">invoice portal</a>?</p><a href="javascript:alert(1)">bad link</a><img src="https://tracker.example/pixel.png" alt="tracking pixel"><script>alert("xss")</script>',
      attachments: [
        {
          id: "att_invoice",
          displayFilename: "invoice.pdf",
          contentType: "application/pdf",
          sizeBytes: 24_576,
          disposition: "attachment",
        },
        {
          id: "att_inline_logo",
          displayFilename: "logo.png",
          contentType: "image/png",
          sizeBytes: 2_048,
          disposition: "inline",
        },
      ],
    },
    {
      id: "msg_deleted",
      accountId: "acct_work",
      folderId: "folder_work_trash",
      sender: "Old Sender",
      senderEmail: "old@example.com",
      subject: "Old deleted message",
      preview: "Already marked deleted on the server.",
      receivedAt: "2026-05-20T08:15:00",
      unread: false,
      flagged: false,
      bodyText: "Already marked deleted on the server.",
      bodyHtml: null,
      attachments: [],
    },
  ],
};

const archiveSearchResult: MailMessage = {
  id: "msg_search_archive",
  accountId: "acct_personal",
  folderId: "folder_personal_archive",
  sender: "Research",
  senderEmail: "research@example.com",
  subject: "Phoenix report",
  preview: "Phoenix appears only in the FTS-backed archive result.",
  receivedAt: "2026-05-20T07:45:00",
  unread: false,
  flagged: false,
  bodyText: "Phoenix appears only in the FTS-backed archive result.",
  bodyHtml: null,
  attachments: [],
};

function createLargeMailFixture(messageCount: number): MailShellData {
  return {
    accounts: [
      {
        id: "acct_bulk",
        label: "Bulk",
        email: "bulk@example.com",
        status: "connected",
        unreadCount: Math.ceil(messageCount / 2),
        signatureHtml: null,
      },
    ],
    folders: [
      {
        id: "folder_bulk_inbox",
        accountId: "acct_bulk",
        name: "Inbox",
        kind: "inbox",
        unreadCount: Math.ceil(messageCount / 2),
        totalCount: messageCount,
      },
    ],
    messages: Array.from({ length: messageCount }, (_, index) => {
      const paddedIndex = index.toString().padStart(5, "0");

      return {
        id: `msg_bulk_${paddedIndex}`,
        accountId: "acct_bulk",
        folderId: "folder_bulk_inbox",
        sender: `Sender ${index}`,
        senderEmail: `sender${index}@example.com`,
        subject: `Bulk message ${paddedIndex}`,
        preview: `Snippet for bulk message ${paddedIndex} remains stable while scrolling.`,
        receivedAt: `2026-05-22T${(index % 24)
          .toString()
          .padStart(2, "0")}:${(index % 60).toString().padStart(2, "0")}:00`,
        unread: index % 2 === 0,
        flagged: index % 10 === 0 || index === messageCount - 1,
        bodyText: `Body for bulk message ${paddedIndex}.`,
        bodyHtml: null,
        attachments: [],
      };
    }),
  };
}

function composerTestHandlers() {
  const saveComposerDraft = vi.fn(
    async (
      request: ComposerDraftSaveRequest,
    ): Promise<ComposerDraftSaveResponse> => ({
      accountId: request.accountId,
      autosaveRevision: 1,
      id: request.draftId,
      state: "active",
    }),
  );
  const discardComposerDraft = vi.fn(
    async (
      request: ComposerDraftDiscardRequest,
    ): Promise<ComposerDraftSaveResponse> => ({
      accountId: request.accountId,
      autosaveRevision: 2,
      id: request.draftId,
      state: "discarded",
    }),
  );
  const stageComposerAttachment = vi.fn(
    async (
      request: ComposerAttachmentStageRequest,
    ): Promise<ComposerAttachment> => ({
      accountId: request.accountId,
      contentId:
        request.disposition === "inline"
          ? `cid_${request.displayFilename.replace(/\W+/g, "_")}`
          : null,
      contentType: request.contentType ?? "application/octet-stream",
      displayFilename: request.displayFilename,
      disposition: request.disposition,
      draftId: request.draftId,
      id: `catt_${request.displayFilename.replace(/\W+/g, "_")}`,
      sizeBytes: request.bytes.length,
    }),
  );
  const sendComposerMail = vi.fn(
    async (request: ComposerSendRequest): Promise<ComposerSendResponse> => ({
      accountId: request.accountId,
      attachmentBytesStreamed: request.attachmentIds.length,
      draftId: request.draftId,
      messageSizeBytes: 512,
      progress: [
        "account-resolved",
        "draft-prepared",
        "auth-resolved",
        "attachment-streamed",
        "mime-composed",
        "smtp-accepted",
        "sent-state-recorded",
        "draft-finalized",
      ],
      providerResponse: "250 accepted",
      rfcMessageId: "<sent@example.com>",
      sentFolderPolicy: "local_receipt_only",
      sentFolderSyncState: "unsupported",
    }),
  );
  const loadRecentRecipients = vi.fn(
    async (
      request: RecentRecipientSearchRequest,
    ): Promise<RecentRecipientSuggestion[]> =>
      [
        {
          accountId: request.accountId,
          address: "ada@example.com",
          id: "rr_ada",
          lastUsedAt: "2026-05-23T10:00:00Z",
          name: "Ada Lovelace",
          useCount: 3,
        },
        {
          accountId: request.accountId,
          address: "copy@example.com",
          id: "rr_copy",
          lastUsedAt: "2026-05-22T10:00:00Z",
          name: "Copy Person",
          useCount: 1,
        },
      ].filter((recipient) =>
        `${recipient.name ?? ""} ${recipient.address}`
          .toLocaleLowerCase()
          .includes(request.query.toLocaleLowerCase()),
      ),
  );

  return {
    discardComposerDraft,
    loadRecentRecipients,
    saveComposerDraft,
    sendComposerMail,
    stageComposerAttachment,
  };
}

describe("App", () => {
  it("renders the accountless mail client shell without fake mailbox data", () => {
    render(<App />);

    expect(
      screen.getByRole("heading", { name: "Komorebi" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Folders" })).toBeVisible();
    expect(screen.getByRole("region", { name: "Messages" })).toBeVisible();
    expect(
      screen.getByRole("region", { name: "Selected message" }),
    ).toBeVisible();
    expect(screen.getByRole("checkbox", { name: "Unified inbox" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Add account" })).toBeDisabled();
    expect(screen.getByText("No accounts configured.")).toBeVisible();
    expect(screen.getByRole("region", { name: "Empty inbox" })).toBeVisible();
    expect(
      screen.getByRole("region", { name: "No selected message" }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Compose" }),
    ).toBeDisabled();
    expect(screen.getByRole("button", { name: "Settings" })).toBeEnabled();
    expect(screen.getByRole("searchbox")).toBeDisabled();
    expect(
      screen.queryByText("Welcome to Tauri + React"),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("personal@example.com")).not.toBeInTheDocument();
    expect(screen.queryByText("Client kickoff agenda")).not.toBeInTheDocument();
  });

  it("supports account, folder, search, reader, composer, and settings workflows", () => {
    render(<App initialData={fixtureMail} />);

    expect(screen.getByRole("checkbox", { name: "Unified inbox" })).toBeChecked();
    expect(screen.getByRole("button", { name: "Compose" })).toBeEnabled();
    expect(screen.getByRole("searchbox")).toBeEnabled();
    expect(screen.getByRole("heading", { name: "Unified Inbox" })).toBeVisible();
    expect(screen.getByText("Roadmap review")).toBeVisible();
    expect(screen.getByText("Invoice question")).toBeVisible();
    expect(screen.getAllByText("2 unread").length).toBeGreaterThan(0);
    expect(screen.getByText("1 unread")).toBeVisible();
    expect(screen.queryByText("Archived receipt")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Roadmap review/ }));
    expect(
      within(screen.getByRole("region", { name: "Selected message" })).getByRole(
        "heading",
        { name: "Roadmap review" },
      ),
    ).toBeVisible();
    expect(
      within(screen.getByRole("region", { name: "Selected message" })).getByText(
        "Review notes for the local cache work.",
      ),
    ).toBeVisible();

    fireEvent.change(screen.getByRole("searchbox"), {
      target: { value: "invoice" },
    });
    expect(screen.getByText("Invoice question")).toBeVisible();
    expect(screen.queryByText("Roadmap review")).not.toBeInTheDocument();

    fireEvent.change(screen.getByRole("searchbox"), {
      target: { value: "" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Personal/ }));
    fireEvent.click(
      within(screen.getByRole("navigation", { name: "Folders" })).getByRole(
        "button",
        { name: /^Archive/ },
      ),
    );
    expect(screen.getByText("Archived receipt")).toBeVisible();
    expect(screen.queryByText("Invoice question")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Compose" }));
    expect(screen.getByRole("region", { name: "Composer" })).toBeVisible();
    expect(screen.getByRole("heading", { name: "New message" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Close" }));

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    const settings = screen.getByRole("region", { name: "Settings" });
    expect(settings).toBeVisible();
    expect(within(settings).getByRole("heading", { name: "Signatures" })).toBeVisible();
    expect(within(settings).getByLabelText("Account")).toHaveValue("acct_personal");
  });

  it("builds a TipTap composer draft, stages attachments, sends, and reports progress", async () => {
    const handlers = composerTestHandlers();
    render(<App initialData={fixtureMail} {...handlers} />);

    fireEvent.click(screen.getByRole("button", { name: "Compose" }));
    const composer = screen.getByRole("dialog", { name: "Composer" });
    expect(
      within(composer).getByText(
        "Drafts stay local to this device until sent or discarded.",
      ),
    ).toBeVisible();

    fireEvent.change(within(composer).getByLabelText("To"), {
      target: { value: "Recipient <recipient@example.com>" },
    });
    fireEvent.change(within(composer).getByLabelText("CC"), {
      target: { value: "copy@example.com" },
    });
    fireEvent.change(within(composer).getByLabelText("BCC"), {
      target: { value: "blind@example.com" },
    });
    fireEvent.change(within(composer).getByLabelText("Subject"), {
      target: { value: "Composer MVP" },
    });

    fireEvent.click(within(composer).getByRole("button", { name: "Insert signature" }));
    fireEvent.click(within(composer).getByRole("button", { name: "Bold" }));
    fireEvent.click(within(composer).getByRole("button", { name: "Bulleted list" }));
    fireEvent.change(within(composer).getByLabelText("Link URL"), {
      target: { value: "https://example.com/follow-up" },
    });
    fireEvent.click(within(composer).getByRole("button", { name: "Apply link" }));

    const attachment = new File(["hello"], "report.txt", { type: "text/plain" });
    fireEvent.change(within(composer).getByLabelText("Attachment picker"), {
      target: { files: [attachment] },
    });
    expect(await within(composer).findByText(/report\.txt - text\/plain - 5 B/)).toBeVisible();

    const inlineImage = new File(
      [new Uint8Array([0x89, 0x50, 0x4e, 0x47, 13, 10, 26, 10])],
      "logo.png",
      { type: "image/png" },
    );
    fireEvent.change(within(composer).getByLabelText("Inline image picker"), {
      target: { files: [inlineImage] },
    });
    expect(await within(composer).findByText(/logo\.png - image\/png - 8 B/)).toBeVisible();

    fireEvent.click(within(composer).getByRole("button", { name: "Save draft" }));
    await waitFor(() => expect(handlers.saveComposerDraft).toHaveBeenCalled());
    const savedRequest =
      handlers.saveComposerDraft.mock.calls[
        handlers.saveComposerDraft.mock.calls.length - 1
      ]?.[0];
    expect(savedRequest).toMatchObject({
      accountId: "acct_personal",
      bcc: [{ address: "blind@example.com", name: null }],
      cc: [{ address: "copy@example.com", name: null }],
      subject: "Composer MVP",
      to: [{ address: "recipient@example.com", name: "Recipient" }],
    });
    expect(savedRequest?.bodyHtml).toContain("Personal signature");
    expect(savedRequest?.attachmentIds).toHaveLength(2);

    fireEvent.click(within(composer).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(handlers.sendComposerMail).toHaveBeenCalledTimes(1));
    const sendRequest = handlers.sendComposerMail.mock.calls[0][0];
    expect(sendRequest.attachmentIds).toEqual(savedRequest?.attachmentIds);
    expect(sendRequest.bodyHtml).toContain("cid:cid_logo_png");
    expect(await within(composer).findByText("SMTP accepted")).toBeVisible();
    expect(within(composer).getByRole("button", { name: "Send" })).toBeDisabled();

    const stageRequest = handlers.stageComposerAttachment.mock.calls[0][0];
    expect(stageRequest).toMatchObject({
      contentType: "text/plain",
      displayFilename: "report.txt",
      disposition: "attachment",
    });
    expect(stageRequest.bytes).toEqual([104, 101, 108, 108, 111]);
  });

  it("loads active local drafts and reopens them without creating a new send draft", async () => {
    const handlers = composerTestHandlers();
    const savedDraft: ComposerDraft = {
      accountId: "acct_personal",
      attachments: [
        {
          accountId: "acct_personal",
          contentId: null,
          contentType: "text/plain",
          displayFilename: "briefing.txt",
          disposition: "attachment",
          draftId: "draft_existing",
          id: "catt_briefing",
          sizeBytes: 5,
        },
      ],
      autosaveRevision: 4,
      bcc: [],
      bodyHtml: "<p>Draft body</p><p>Personal signature</p>",
      bodyText: "Draft body\nPersonal signature",
      cc: [{ address: "copy@example.com", name: null }],
      id: "draft_existing",
      subject: "Budget update",
      to: [{ address: "ada@example.com", name: "Ada Lovelace" }],
    };
    const loadComposerDrafts = vi
      .fn<() => Promise<ComposerDraft[]>>()
      .mockResolvedValueOnce([savedDraft])
      .mockResolvedValue([]);

    render(
      <App
        initialData={fixtureMail}
        loadComposerDrafts={loadComposerDrafts}
        {...handlers}
      />,
    );

    const draftsButton = await screen.findByRole("button", {
      name: "Drafts, 1 saved locally",
    });
    fireEvent.click(draftsButton);

    const draftsDialog = screen.getByRole("dialog", { name: "Local drafts" });
    expect(within(draftsDialog).getByText("Budget update")).toBeVisible();
    expect(within(draftsDialog).getByText(/Ada Lovelace/)).toBeVisible();
    expect(within(draftsDialog).getByText("1 attachment")).toBeVisible();

    fireEvent.click(
      within(draftsDialog).getByRole("button", {
        name: "Open draft: Budget update",
      }),
    );

    const composer = screen.getByRole("dialog", { name: "Composer" });
    expect(within(composer).getByLabelText("From")).toHaveValue("acct_personal");
    expect(within(composer).getByLabelText("From")).toBeDisabled();
    expect(within(composer).getByLabelText("To")).toHaveValue(
      "Ada Lovelace <ada@example.com>",
    );
    expect(within(composer).getByLabelText("CC")).toHaveValue("copy@example.com");
    expect(within(composer).getByLabelText("Subject")).toHaveValue(
      "Budget update",
    );
    expect(
      within(composer).getByRole("textbox", { name: "Message body" }),
    ).toHaveTextContent("Draft body");
    expect(
      within(composer).getByRole("textbox", { name: "Message body" }),
    ).toHaveTextContent("Personal signature");
    expect(
      within(composer).getByText(/briefing\.txt - text\/plain - 5 B/),
    ).toBeVisible();

    fireEvent.click(within(composer).getByRole("button", { name: "Send" }));
    await waitFor(() => expect(handlers.sendComposerMail).toHaveBeenCalledTimes(1));
    expect(handlers.sendComposerMail.mock.calls[0][0]).toMatchObject({
      attachmentIds: ["catt_briefing"],
      draftId: "draft_existing",
      subject: "Budget update",
      to: [{ address: "ada@example.com", name: "Ada Lovelace" }],
    });
    expect(await within(composer).findByText("SMTP accepted")).toBeVisible();
  });

  it("shows composer validation errors before send", async () => {
    const handlers = composerTestHandlers();
    render(<App initialData={fixtureMail} {...handlers} />);

    fireEvent.click(screen.getByRole("button", { name: "Compose" }));
    const composer = screen.getByRole("dialog", { name: "Composer" });

    fireEvent.change(within(composer).getByLabelText("To"), {
      target: { value: "not-an-address" },
    });
    fireEvent.click(within(composer).getByRole("button", { name: "Send" }));

    const alert = await within(composer).findByRole("alert");
    expect(alert).toHaveTextContent("To: not-an-address is not a valid email address.");
    expect(alert).toHaveTextContent("Add at least one recipient.");
    expect(alert).toHaveTextContent("Add a message body.");
    expect(handlers.sendComposerMail).not.toHaveBeenCalled();
  });

  it("maps SMTP send failures to a redacted composer error", async () => {
    const handlers = composerTestHandlers();
    handlers.sendComposerMail.mockRejectedValueOnce(
      new Error("SMTP AUTH failed for access-token-super-secret"),
    );
    render(<App initialData={fixtureMail} {...handlers} />);

    fireEvent.click(screen.getByRole("button", { name: "Compose" }));
    const composer = screen.getByRole("dialog", { name: "Composer" });
    fireEvent.change(within(composer).getByLabelText("To"), {
      target: { value: "Recipient <recipient@example.com>" },
    });
    fireEvent.click(within(composer).getByRole("button", { name: "Insert signature" }));
    fireEvent.click(within(composer).getByRole("button", { name: "Send" }));

    const alert = await within(composer).findByRole("alert");
    expect(alert).toHaveTextContent(
      "Message could not be sent. The draft remains saved.",
    );
    expect(alert).not.toHaveTextContent("access-token-super-secret");
    expect(await within(composer).findByText("Composer needs attention")).toBeVisible();
  });

  it("autocompletes recent recipients with the keyboard", async () => {
    const handlers = composerTestHandlers();
    render(<App initialData={fixtureMail} {...handlers} />);

    fireEvent.click(screen.getByRole("button", { name: "Compose" }));
    const composer = screen.getByRole("dialog", { name: "Composer" });
    const toInput = within(composer).getByLabelText("To");

    fireEvent.focus(toInput);
    fireEvent.change(toInput, { target: { value: "ada" } });
    expect(
      await within(composer).findByRole("option", { name: /Ada Lovelace/ }),
    ).toBeVisible();

    fireEvent.keyDown(toInput, { key: "Enter" });

    expect(toInput).toHaveValue("Ada Lovelace <ada@example.com>, ");
    expect(handlers.loadRecentRecipients).toHaveBeenCalledWith(
      expect.objectContaining({
        accountId: "acct_personal",
        query: "ada",
      }),
      expect.any(AbortSignal),
    );
  });

  it("deduplicates composer recipients while keeping malformed-address errors clear", async () => {
    const handlers = composerTestHandlers();
    render(<App initialData={fixtureMail} {...handlers} />);

    fireEvent.click(screen.getByRole("button", { name: "Compose" }));
    const composer = screen.getByRole("dialog", { name: "Composer" });

    fireEvent.change(within(composer).getByLabelText("To"), {
      target: {
        value: "Duplicate <duplicate@example.com>, duplicate@example.com",
      },
    });
    fireEvent.change(within(composer).getByLabelText("CC"), {
      target: { value: "DUPLICATE@example.com, copy@example.com" },
    });
    fireEvent.change(within(composer).getByLabelText("BCC"), {
      target: { value: "bad-address, copy@example.com" },
    });
    fireEvent.click(within(composer).getByRole("button", { name: "Insert signature" }));

    fireEvent.click(within(composer).getByRole("button", { name: "Send" }));

    const alert = await within(composer).findByRole("alert");
    expect(alert).toHaveTextContent("BCC: bad-address is not a valid email address.");
    expect(handlers.sendComposerMail).not.toHaveBeenCalled();

    fireEvent.change(within(composer).getByLabelText("BCC"), {
      target: { value: "copy@example.com" },
    });
    fireEvent.click(within(composer).getByRole("button", { name: "Send" }));

    await waitFor(() => expect(handlers.sendComposerMail).toHaveBeenCalledTimes(1));
    const sendRequest = handlers.sendComposerMail.mock.calls[0][0];
    expect(sendRequest.to).toEqual([
      { address: "duplicate@example.com", name: "Duplicate" },
    ]);
    expect(sendRequest.cc).toEqual([{ address: "copy@example.com", name: null }]);
    expect(sendRequest.bcc).toEqual([]);
    expect(
      within(composer)
        .getAllByRole("status")
        .some((status) =>
          status.textContent?.includes("Duplicate recipient skipped"),
        ),
    ).toBe(true);
  });

  it("renders selected message detail through safe body and attachment boundaries", () => {
    const { container } = render(<App initialData={fixtureMail} />);

    fireEvent.click(screen.getByRole("button", { name: /Invoice question/ }));

    const reader = screen.getByRole("region", { name: "Selected message" });
    expect(
      within(reader).getByRole("heading", { name: "Invoice question" }),
    ).toBeVisible();
    expect(reader).toHaveTextContent(
      "Can you confirm the invoice number in the invoice portal?",
    );
    expect(within(reader).getByText("Remote content blocked")).toBeVisible();
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("img[src]")).toBeNull();

    expect(
      within(reader).getByRole("button", {
        name: "Reply unavailable",
      }),
    ).toBeDisabled();
    expect(
      within(reader).getByRole("button", {
        name: "Forward unavailable",
      }),
    ).toBeDisabled();
    expect(
      within(reader).getByRole("button", {
        name: "Load remote content unavailable; remote resources are blocked by default",
      }),
    ).toBeDisabled();

    const attachments = within(reader).getByRole("region", {
      name: "Attachments",
    });
    expect(within(attachments).getByText("invoice.pdf")).toBeVisible();
    expect(within(attachments).getByText("application/pdf - 24 KB")).toBeVisible();
    expect(within(attachments).getByText("logo.png")).toBeVisible();
    expect(within(attachments).getByText("image/png - 2 KB")).toBeVisible();
    expect(
      within(attachments).getByRole("button", {
        name: "Preview invoice.pdf unavailable",
      }),
    ).toBeDisabled();
    expect(
      within(attachments).getByRole("button", {
        name: "Save invoice.pdf unavailable",
      }),
    ).toBeDisabled();
    expect(
      within(attachments).getByRole("button", {
        name: "Download invoice.pdf unavailable",
      }),
    ).toBeDisabled();
    expect(reader).not.toHaveTextContent("AppData");
    expect(reader).not.toHaveTextContent("att_invoice.pdf");
  });

  it("opens safe message links through the typed command and leaves unsafe links inert", async () => {
    const openExternalLink = vi.fn(
      async (
        request: ExternalLinkOpenRequest,
      ): Promise<ExternalLinkOpenResponse> => ({
        opened: true,
        openedUrl: request.url,
      }),
    );

    render(
      <App initialData={fixtureMail} openExternalLink={openExternalLink} />,
    );

    fireEvent.click(screen.getByRole("button", { name: /Invoice question/ }));
    const reader = screen.getByRole("region", { name: "Selected message" });

    fireEvent.click(
      within(reader).getByRole("link", { name: "invoice portal" }),
    );

    await waitFor(() =>
      expect(openExternalLink).toHaveBeenCalledWith({
        url: "https://example.com/invoice",
        userRequested: true,
      }),
    );
    expect(await within(reader).findByText("External link opened.")).toBeVisible();

    fireEvent.click(within(reader).getByText("bad link"));
    expect(openExternalLink).toHaveBeenCalledTimes(1);
  });

  it("routes move, archive, delete, mark-read, expunge, undo, and retry through typed mail actions", async () => {
    const performMailAction = vi.fn(
      async (request: MailActionRequest): Promise<MailActionResponse> => {
        if (request.action.kind === "archive" && request.attempt !== 2) {
          throw {
            reason: "mail server operation can retry",
            retry: { kind: "retry-now", nextAttempt: 2, maxAttempts: 3 },
          };
        }

        if (request.action.kind === "mark-read") {
          return {
            accountId: request.accountId,
            action: request.action,
            mutation: { kind: "flags-updated", read: request.action.read },
            retry: { kind: "retryable", maxAttempts: 3 },
            undo: {
              accountId: request.accountId,
              action: { kind: "mark-read", read: !request.action.read },
              folderId: request.folderId,
              messageId: request.messageId ?? "msg_invoice",
            },
          };
        }

        if (request.action.kind === "expunge-deleted") {
          return {
            accountId: request.accountId,
            action: request.action,
            mutation: { kind: "expunged", folderId: request.folderId, removedCount: 1 },
            retry: { kind: "retryable", maxAttempts: 3 },
            undo: null,
          };
        }

        const targetFolderId =
          request.action.kind === "move"
            ? request.action.targetFolderId
            : request.action.kind === "archive"
              ? "folder_work_archive"
              : "folder_work_trash";

        return {
          accountId: request.accountId,
          action: request.action,
          mutation: {
            kind: "moved",
            fromFolderId: request.folderId,
            toFolderId: targetFolderId,
          },
          retry: { kind: "retryable", maxAttempts: 3 },
          undo: {
            accountId: request.accountId,
            action: { kind: "move", targetFolderId: request.folderId },
            folderId: targetFolderId,
            messageId: request.messageId ?? "msg_invoice",
          },
        };
      },
    );

    render(
      <App initialData={fixtureMail} performMailAction={performMailAction} />,
    );

    fireEvent.click(screen.getByRole("button", { name: /Invoice question/ }));
    let reader = screen.getByRole("region", { name: "Selected message" });

    fireEvent.click(within(reader).getByRole("button", { name: "Mark read" }));
    await waitFor(() =>
      expect(performMailAction).toHaveBeenCalledWith({
        accountId: "acct_work",
        action: { kind: "mark-read", read: true },
        confirmed: false,
        folderId: "folder_work_inbox",
        messageId: "msg_invoice",
        userRequested: true,
      }),
    );
    expect(await within(reader).findByText("Message marked read.")).toBeVisible();

    fireEvent.change(
      within(reader).getByLabelText("Move message to folder"),
      { target: { value: "folder_work_archive" } },
    );
    await waitFor(() =>
      expect(performMailAction).toHaveBeenLastCalledWith({
        accountId: "acct_work",
        action: { kind: "move", targetFolderId: "folder_work_archive" },
        confirmed: false,
        folderId: "folder_work_inbox",
        messageId: "msg_invoice",
        userRequested: true,
      }),
    );
    expect(await within(reader).findByText("Message moved.")).toBeVisible();

    fireEvent.click(within(reader).getByRole("button", { name: "Undo" }));
    await waitFor(() =>
      expect(performMailAction).toHaveBeenLastCalledWith({
        accountId: "acct_work",
        action: { kind: "move", targetFolderId: "folder_work_inbox" },
        confirmed: false,
        folderId: "folder_work_archive",
        messageId: "msg_invoice",
        userRequested: true,
      }),
    );

    fireEvent.click(within(reader).getByRole("button", { name: "Archive" }));
    expect(
      await within(reader).findByText("mail server operation can retry"),
    ).toBeVisible();
    fireEvent.click(within(reader).getByRole("button", { name: "Retry" }));
    await waitFor(() =>
      expect(performMailAction).toHaveBeenLastCalledWith({
        accountId: "acct_work",
        action: { kind: "archive" },
        attempt: 2,
        confirmed: false,
        folderId: "folder_work_inbox",
        messageId: "msg_invoice",
        userRequested: true,
      }),
    );
    expect(await within(reader).findByText("Message archived.")).toBeVisible();

    fireEvent.click(within(reader).getByRole("button", { name: "Undo" }));
    await waitFor(() =>
      expect(performMailAction).toHaveBeenLastCalledWith({
        accountId: "acct_work",
        action: { kind: "move", targetFolderId: "folder_work_inbox" },
        confirmed: false,
        folderId: "folder_work_archive",
        messageId: "msg_invoice",
        userRequested: true,
      }),
    );

    const callCountBeforeDelete = performMailAction.mock.calls.length;
    fireEvent.click(within(reader).getByRole("button", { name: "Delete" }));
    expect(
      within(reader).getByText("Delete moves this message to Trash and can be undone while the message remains there."),
    ).toBeVisible();
    expect(performMailAction).toHaveBeenCalledTimes(callCountBeforeDelete);
    fireEvent.click(within(reader).getByRole("button", { name: "Move to Trash" }));
    await waitFor(() =>
      expect(performMailAction).toHaveBeenLastCalledWith({
        accountId: "acct_work",
        action: { kind: "delete-to-trash" },
        confirmed: true,
        folderId: "folder_work_inbox",
        messageId: "msg_invoice",
        userRequested: true,
      }),
    );
    expect(await within(reader).findByText("Message moved to Trash.")).toBeVisible();

    fireEvent.click(within(reader).getByRole("button", { name: "Undo" }));
    await waitFor(() =>
      expect(performMailAction).toHaveBeenLastCalledWith({
        accountId: "acct_work",
        action: { kind: "move", targetFolderId: "folder_work_inbox" },
        confirmed: false,
        folderId: "folder_work_trash",
        messageId: "msg_invoice",
        userRequested: true,
      }),
    );

    fireEvent.click(
      screen.getByRole("button", {
        name: "Trash folder, 0 unread, 1 messages",
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: /Old deleted message/ }));
    reader = screen.getByRole("region", { name: "Selected message" });
    fireEvent.click(within(reader).getByRole("button", { name: "Expunge" }));
    expect(
      within(reader).getByText(
        "Expunge permanently removes messages already marked deleted in this folder. This cannot be undone.",
      ),
    ).toBeVisible();
    fireEvent.click(
      within(within(reader).getByRole("region", { name: "Confirm expunge" }))
        .getByRole("button", { name: "Expunge" }),
    );
    await waitFor(() =>
      expect(performMailAction).toHaveBeenLastCalledWith({
        accountId: "acct_work",
        action: { kind: "expunge-deleted" },
        confirmed: true,
        folderId: "folder_work_trash",
        messageId: null,
        userRequested: true,
      }),
    );
    expect(
      await within(reader).findByText("Expunged 1 deleted messages."),
    ).toBeVisible();
  });

  it("persists narrow remote-content preferences and renders consented remote images", async () => {
    const allowRemoteContent = vi.fn(
      async (
        request: RemoteContentPreferenceRequest,
      ): Promise<RemoteContentPreferenceResponse> => ({
        accountId: request.accountId,
        messageId: request.messageId,
        remoteContentAllowed: true,
        scope: request.scope,
        senderEmail:
          request.scope === "sender" ? "finance@example.com" : null,
      }),
    );

    render(
      <App
        allowRemoteContent={allowRemoteContent}
        initialData={fixtureMail}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /Invoice question/ }));
    const reader = screen.getByRole("region", { name: "Selected message" });
    expect(reader.querySelector("img[src]")).toBeNull();

    fireEvent.click(
      within(reader).getByRole("button", { name: "Load remote content" }),
    );
    const preferencePanel = within(reader).getByRole("region", {
      name: "Remote content preference",
    });
    expect(
      within(preferencePanel).getByText(
        "Remote images can reveal when and where mail is opened.",
      ),
    ).toBeVisible();

    fireEvent.click(
      within(preferencePanel).getByRole("button", {
        name: "Allow this message",
      }),
    );

    await waitFor(() =>
      expect(allowRemoteContent).toHaveBeenCalledWith({
        accountId: "acct_work",
        folderId: "folder_work_inbox",
        messageId: "msg_invoice",
        scope: "message",
        userRequested: true,
      }),
    );
    expect(
      (
        await within(reader).findAllByText(
          "Remote content allowed for this message",
        )
      ).length,
    ).toBeGreaterThan(0);
    const remoteImage = reader.querySelector<HTMLImageElement>(
      'img[alt="tracking pixel"]',
    );
    expect(remoteImage?.getAttribute("src")).toBe(
      "https://tracker.example/pixel.png",
    );
    expect(reader.querySelector("script")).toBeNull();
  });

  it("routes reader attachment preview, save, and download through typed commands", async () => {
    const expectedRequest: AttachmentCommandRequest = {
      accountId: "acct_work",
      folderId: "folder_work_inbox",
      messageId: "msg_invoice",
      attachmentId: "att_inline_logo",
      userRequested: true,
    };
    const previewAttachment = vi.fn(
      async (
        request: AttachmentCommandRequest,
      ): Promise<AttachmentPreviewResponse> => ({
        attachmentId: request.attachmentId,
        autoOpen: false,
        bytes: [137, 80, 78, 71],
        contentType: "image/png",
        displayFilename: "logo.png",
        kind: "raster-image",
        sizeBytes: 4,
        text: null,
      }),
    );
    const saveAttachment = vi.fn(
      async (
        request: AttachmentCommandRequest,
      ): Promise<AttachmentSaveResponse> => ({
        attachmentId: request.attachmentId,
        bytesWritten: 4,
        contentType: "image/png",
        destinationLabel: "logo.png",
        displayFilename: "logo.png",
        savedFilename: "logo.png",
        sizeBytes: 4,
      }),
    );
    const downloadAttachment = vi.fn(
      async (
        request: AttachmentCommandRequest,
      ): Promise<AttachmentDownloadResponse> => ({
        attachmentId: request.attachmentId,
        bytes: [137, 80, 78, 71],
        contentType: "image/png",
        displayFilename: "logo.png",
        sizeBytes: 4,
      }),
    );
    const originalCreateObjectUrl = URL.createObjectURL;
    const originalRevokeObjectUrl = URL.revokeObjectURL;
    const createObjectURL = vi.fn(() => "blob:komorebi-attachment");
    const revokeObjectURL = vi.fn();
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: createObjectURL,
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      configurable: true,
      value: revokeObjectURL,
    });
    const anchorClick = vi
      .spyOn(HTMLAnchorElement.prototype, "click")
      .mockImplementation(() => undefined);

    try {
      render(
        <App
          downloadAttachment={downloadAttachment}
          initialData={fixtureMail}
          previewAttachment={previewAttachment}
          saveAttachment={saveAttachment}
        />,
      );

      fireEvent.click(screen.getByRole("button", { name: /Invoice question/ }));
      const reader = screen.getByRole("region", { name: "Selected message" });
      const attachments = within(reader).getByRole("region", {
        name: "Attachments",
      });
      expect(
        within(attachments).getByRole("button", {
          name: "Preview invoice.pdf unavailable",
        }),
      ).toBeDisabled();

      fireEvent.click(
        within(attachments).getByRole("button", { name: "Preview logo.png" }),
      );
      await waitFor(() =>
        expect(previewAttachment).toHaveBeenCalledWith(expectedRequest),
      );
      const preview = within(reader).getByRole("region", {
        name: "Attachment preview",
      });
      expect(within(preview).getByRole("img", { name: "logo.png" })).toBeVisible();

      fireEvent.click(
        within(attachments).getByRole("button", { name: "Save logo.png" }),
      );
      await waitFor(() =>
        expect(saveAttachment).toHaveBeenCalledWith(expectedRequest),
      );
      expect(attachments).toHaveTextContent("Saved logo.png to logo.png.");

      fireEvent.click(
        within(attachments).getByRole("button", { name: "Download logo.png" }),
      );
      await waitFor(() =>
        expect(downloadAttachment).toHaveBeenCalledWith(expectedRequest),
      );
      expect(createObjectURL).toHaveBeenCalledTimes(1);
      expect(anchorClick).toHaveBeenCalledTimes(1);
      expect(revokeObjectURL).toHaveBeenCalledWith("blob:komorebi-attachment");
      expect(attachments).toHaveTextContent("Download prepared for logo.png.");
    } finally {
      anchorClick.mockRestore();
      if (originalCreateObjectUrl) {
        Object.defineProperty(URL, "createObjectURL", {
          configurable: true,
          value: originalCreateObjectUrl,
        });
      } else {
        delete (URL as unknown as { createObjectURL?: unknown }).createObjectURL;
      }
      if (originalRevokeObjectUrl) {
        Object.defineProperty(URL, "revokeObjectURL", {
          configurable: true,
          value: originalRevokeObjectUrl,
        });
      } else {
        delete (URL as unknown as { revokeObjectURL?: unknown }).revokeObjectURL;
      }
    }
  });

  it("debounces and cancels FTS-backed search while identifying result account and folder", async () => {
    vi.useFakeTimers();

    const abortedQueries: string[] = [];
    const pendingSearches: Array<{
      query: string;
      resolve: (response: MailSearchResponse) => void;
    }> = [];
    const searchMail = vi.fn((request: MailSearchRequest, signal: AbortSignal) => {
      signal.addEventListener("abort", () => {
        abortedQueries.push(request.query);
      });

      return new Promise<MailSearchResponse>((resolve) => {
        pendingSearches.push({ query: request.query, resolve });
      });
    });

    try {
      render(
        <App
          initialData={fixtureMail}
          searchDebounceMs={200}
          searchMail={searchMail}
        />,
      );

      fireEvent.change(screen.getByRole("searchbox"), {
        target: { value: "pho" },
      });

      act(() => {
        vi.advanceTimersByTime(199);
      });
      expect(searchMail).not.toHaveBeenCalled();

      act(() => {
        vi.advanceTimersByTime(1);
      });
      expect(searchMail).toHaveBeenCalledTimes(1);
      expect(pendingSearches[0].query).toBe("pho");

      fireEvent.change(screen.getByRole("searchbox"), {
        target: { value: "phoenix" },
      });
      expect(abortedQueries).toContain("pho");

      act(() => {
        vi.advanceTimersByTime(200);
      });
      expect(searchMail).toHaveBeenCalledTimes(2);
      expect(pendingSearches[1].query).toBe("phoenix");

      await act(async () => {
        pendingSearches[1].resolve({
          indexStatus: "ready",
          query: "phoenix",
          results: [archiveSearchResult],
        });
      });

      expect(screen.getByText("Phoenix report")).toBeVisible();
      expect(screen.getByText("Personal / Archive")).toBeVisible();
      expect(screen.queryByText("Roadmap review")).not.toBeInTheDocument();

      fireEvent.click(screen.getByRole("button", { name: /Phoenix report/ }));
      expect(
        within(screen.getByRole("region", { name: "Selected message" })).getByRole(
          "heading",
          { name: "Phoenix report" },
        ),
      ).toBeVisible();
    } finally {
      vi.useRealTimers();
    }
  });

  it("shows the no-index state when backend FTS is unavailable", async () => {
    const searchMail = vi.fn().mockResolvedValue({
      indexStatus: "unavailable",
      query: "invoice",
      results: [],
    } satisfies MailSearchResponse);

    render(
      <App
        initialData={fixtureMail}
        searchDebounceMs={0}
        searchMail={searchMail}
      />,
    );

    fireEvent.change(screen.getByRole("searchbox"), {
      target: { value: "invoice" },
    });

    expect(
      await screen.findByRole("region", { name: "Search index unavailable" }),
    ).toBeVisible();
    expect(searchMail).toHaveBeenCalledWith(
      {
        accountId: null,
        folderId: null,
        limit: 100,
        query: "invoice",
      },
      expect.any(AbortSignal),
    );
  });

  it("supports keyboard navigation for search, folders, messages, and reading", () => {
    render(<App initialData={fixtureMail} />);

    const shell = screen.getByRole("main");
    const search = screen.getByRole("searchbox");

    fireEvent.keyDown(shell, { key: "/" });
    expect(search).toHaveFocus();

    const roadmap = screen.getByRole("button", { name: /Roadmap review/ });
    roadmap.focus();
    fireEvent.keyDown(roadmap, { key: "ArrowDown" });

    const invoice = screen.getByRole("button", { name: /Invoice question/ });
    expect(invoice).toHaveFocus();
    expect(
      within(screen.getByRole("region", { name: "Selected message" })).getByRole(
        "heading",
        { name: "Invoice question" },
      ),
    ).toBeVisible();

    fireEvent.keyDown(invoice, { key: "Enter" });
    expect(screen.getByRole("region", { name: "Selected message" })).toHaveFocus();

    const folders = screen.getByRole("navigation", { name: "Folders" });
    const unifiedInbox = within(folders).getByRole("button", {
      name: /Unified Inbox folder/,
    });
    unifiedInbox.focus();
    fireEvent.keyDown(unifiedInbox, { key: "ArrowDown" });

    const personalInbox = within(folders).getAllByRole("button", {
      name: /^Inbox folder/,
    })[0];
    expect(personalInbox).toHaveFocus();
    expect(screen.getByText("Roadmap review")).toBeVisible();
    expect(screen.queryByText("Archived receipt")).not.toBeInTheDocument();
  });

  it("keeps composer and settings dialogs keyboard reachable with focus return", async () => {
    render(<App initialData={fixtureMail} />);

    const composeButton = screen.getByRole("button", { name: "Compose" });
    composeButton.focus();
    fireEvent.click(composeButton);

    const composer = screen.getByRole("dialog", { name: "Composer" });
    await waitFor(() => {
      expect(within(composer).getByLabelText("To")).toHaveFocus();
    });

    within(composer).getByRole("button", { name: "Close" }).focus();
    fireEvent.keyDown(composer, { key: "Tab", shiftKey: true });
    expect(within(composer).getByRole("button", { name: "Send" })).toHaveFocus();
    fireEvent.keyDown(composer, { key: "Tab" });
    expect(within(composer).getByRole("button", { name: "Close" })).toHaveFocus();

    fireEvent.keyDown(composer, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "Composer" })).not.toBeInTheDocument();
    expect(composeButton).toHaveFocus();

    const settingsButton = screen.getByRole("button", { name: "Settings" });
    settingsButton.focus();
    fireEvent.click(settingsButton);

    const settings = screen.getByRole("dialog", { name: "Settings" });
    expect(within(settings).getByRole("heading", { name: "Signatures" })).toBeVisible();

    fireEvent.keyDown(settings, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "Settings" })).not.toBeInTheDocument();
    expect(settingsButton).toHaveFocus();
  });

  it("virtualizes 10,000 message rows while preserving row state during scroll", async () => {
    render(<App initialData={createLargeMailFixture(10_000)} />);

    const messageList = screen.getByTestId("message-virtual-list");
    expect(messageList).toHaveAttribute("data-message-count", "10000");

    const initiallyRenderedMessages = within(messageList).getAllByRole("button", {
      name: /message from Sender/,
    });
    expect(initiallyRenderedMessages.length).toBeGreaterThan(0);
    expect(initiallyRenderedMessages.length).toBeLessThan(80);

    const firstMessage = screen.getByRole("button", {
      name: /Bulk message 00000/,
    });
    expect(firstMessage).toHaveTextContent("Sender 0");
    expect(firstMessage).toHaveTextContent("Flagged");
    expect(firstMessage).toHaveTextContent(
      formatDateTime("2026-05-22T00:00:00", "en"),
    );
    expect(firstMessage).toHaveTextContent(
      "Snippet for bulk message 00000 remains stable while scrolling.",
    );

    fireEvent.click(firstMessage);
    expect(firstMessage).toHaveAttribute("aria-current", "true");

    fireEvent.keyDown(firstMessage, { key: "End" });

    const lastMessage = await screen.findByRole("button", {
      name: /Bulk message 09999/,
    });
    await waitFor(() => expect(lastMessage).toHaveFocus());
    expect(lastMessage).toHaveAttribute("aria-current", "true");
    expect(lastMessage).toHaveTextContent("Sender 9999");
    expect(lastMessage).toHaveTextContent("Flagged");
    expect(lastMessage).toHaveTextContent(
      formatDateTime("2026-05-22T15:39:00", "en"),
    );
    expect(lastMessage).toHaveTextContent(
      "Snippet for bulk message 09999 remains stable while scrolling.",
    );
    expect(
      within(screen.getByRole("region", { name: "Selected message" })).getByRole(
        "heading",
        { name: "Bulk message 09999" },
      ),
    ).toBeVisible();
    expect(
      within(messageList).getAllByRole("button", {
        name: /message from Sender/,
      }).length,
    ).toBeLessThan(80);
    expect(screen.queryByText("Bulk message 05000")).not.toBeInTheDocument();
  });

  it("shows account sync detail and launches OAuth reconnect from the status surface", async () => {
    const statusMail: MailShellData = {
      ...fixtureMail,
      accounts: [
        {
          ...fixtureMail.accounts[0],
          authKind: "oauth2",
          lastSuccessfulSyncAt: "2026-05-23T15:00:00Z",
          providerKind: "gmail",
          status: "reconnect-needed",
          statusAction: "reauthenticate",
          statusMessage: "Reauthentication is required for this account.",
          statusUpdatedAt: "2026-05-23T16:01:00Z",
        },
      ],
      folders: fixtureMail.folders.filter(
        (folder) => folder.accountId === "acct_personal",
      ),
      messages: fixtureMail.messages.filter(
        (message) => message.accountId === "acct_personal",
      ),
    };
    const startOAuthAccountReauth = vi.fn().mockResolvedValue({
      account: {
        authKind: "oauth2",
        authStatus: "connected",
        displayEmail: "person@example.com",
        displayName: "Personal",
        id: "acct_personal",
        providerKind: "gmail",
        username: "person@example.com",
      },
      credentialStored: true,
    });

    render(
      <App
        initialData={statusMail}
        startOAuthAccountReauth={startOAuthAccountReauth}
      />,
    );

    expect(screen.getAllByText(/Last sync:/).length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: "Reconnect" }));

    const statusDialog = screen.getByRole("dialog", { name: "Account status" });
    expect(
      within(statusDialog).getByText(
        "Reauthentication is required for this account.",
      ),
    ).toBeVisible();
    expect(within(statusDialog).getByText("Last successful sync")).toBeVisible();

    fireEvent.click(
      within(statusDialog).getByRole("button", { name: "Reconnect" }),
    );

    await waitFor(() =>
      expect(startOAuthAccountReauth).toHaveBeenCalledWith({
        accountId: "acct_personal",
        clientId: null,
      }),
    );
  });

  it("uses the generic credential path when reconnecting a generic account from status detail", async () => {
    const statusMail: MailShellData = {
      accounts: [
        {
          authKind: "app_password",
          email: "generic@example.com",
          id: "acct_generic",
          label: "Generic",
          providerKind: "generic_imap_smtp",
          signatureHtml: null,
          status: "error",
          statusAction: "reauthenticate",
          statusMessage: "Reauthentication is required for this account.",
          unreadCount: 0,
        },
      ],
      folders: [],
      messages: [],
    };
    const reauthAccount = vi.fn().mockResolvedValue({
      account: {
        authKind: "app_password",
        authStatus: "connected",
        displayEmail: "generic@example.com",
        displayName: "Generic",
        id: "acct_generic",
        providerKind: "generic_imap_smtp",
        username: "generic@example.com",
      },
      credentialStored: true,
    });

    render(<App initialData={statusMail} reauthAccount={reauthAccount} />);

    fireEvent.click(screen.getByRole("button", { name: "Reconnect" }));
    const statusDialog = screen.getByRole("dialog", { name: "Account status" });
    fireEvent.change(within(statusDialog).getByLabelText("Replacement secret"), {
      target: { value: "new-app-password" },
    });
    fireEvent.click(
      within(statusDialog).getByRole("button", { name: "Reconnect" }),
    );

    await waitFor(() =>
      expect(reauthAccount).toHaveBeenCalledWith({
        accountId: "acct_generic",
        secret: "new-app-password",
      }),
    );
  });

  it("loads real shell data through the cache data source and preserves selection when scope still contains it", async () => {
    const loadMailShellData = vi.fn().mockResolvedValue(fixtureMail);

    render(<App loadMailShellData={loadMailShellData} />);

    expect(screen.getByText("Loading accounts")).toBeVisible();
    expect(await screen.findByText("Roadmap review")).toBeVisible();
    expect(loadMailShellData).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: /Roadmap review/ }));
    expect(
      within(screen.getByRole("region", { name: "Selected message" })).getByRole(
        "heading",
        { name: "Roadmap review" },
      ),
    ).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: /Personal/ }));
    expect(
      within(screen.getByRole("region", { name: "Selected message" })).getByRole(
        "heading",
        { name: "Roadmap review" },
      ),
    ).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: /Work/ }));
    expect(screen.getByText("Invoice question")).toBeVisible();
    expect(screen.queryByRole("heading", { name: "Roadmap review" })).not.toBeInTheDocument();
    expect(
      screen.getByRole("region", { name: "No selected message" }),
    ).toBeVisible();
  });

  it("refreshes the shell snapshot when the backend sync status event stream updates", async () => {
    let statusHandler: ((payload: unknown) => void) | null = null;
    const updatedMail: MailShellData = {
      ...fixtureMail,
      accounts: fixtureMail.accounts.map((account) =>
        account.id === "acct_personal"
          ? {
              ...account,
              lastSuccessfulSyncAt: "2026-05-23T15:00:00Z",
              status: "reconnect-needed",
              statusAction: "reauthenticate",
              statusMessage: "Reauthentication is required for this account.",
              statusUpdatedAt: "2026-05-23T16:01:00Z",
            }
          : account,
      ),
    };
    const loadMailShellData = vi
      .fn()
      .mockResolvedValueOnce(fixtureMail)
      .mockResolvedValueOnce(updatedMail);
    const unsubscribe = vi.fn();
    const subscribeToSyncStatus = vi.fn((handler: (payload: unknown) => void) => {
      statusHandler = handler;
      return unsubscribe;
    });

    render(
      <App
        loadMailShellData={loadMailShellData}
        subscribeToSyncStatus={subscribeToSyncStatus}
      />,
    );

    expect(await screen.findByText("Roadmap review")).toBeVisible();
    expect(subscribeToSyncStatus).toHaveBeenCalledTimes(1);

    act(() => {
      statusHandler?.({
        accountId: "acct_personal",
        status: { state: "error", updatedAt: 1779552060 },
      });
    });

    await waitFor(() => expect(loadMailShellData).toHaveBeenCalledTimes(2));
    expect(screen.getAllByText(/Last sync:/).length).toBeGreaterThan(0);
  });

  it("saves account settings through the typed command and reloads the cache snapshot", async () => {
    const updatedMail: MailShellData = {
      ...fixtureMail,
      accounts: fixtureMail.accounts.map((account) =>
        account.id === "acct_personal"
          ? {
              ...account,
              displayName: "Sender",
              label: "Sender",
              localPreferences: { includeInUnifiedInbox: false },
              signatureHtml: "<p>Regards</p>",
            }
          : account,
      ),
    };
    const loadMailShellData = vi
      .fn()
      .mockResolvedValueOnce(fixtureMail)
      .mockResolvedValueOnce(updatedMail);
    const updateAccountSettings = vi.fn().mockResolvedValue({
      account: {
        authKind: "oauth2",
        authStatus: "connected",
        displayEmail: "person@example.com",
        displayName: "Sender",
        id: "acct_personal",
        providerKind: "gmail",
        username: "person@example.com",
      },
      localPreferences: { includeInUnifiedInbox: false },
      signatureHtml: "<p>Regards</p>",
    });

    render(
      <App
        loadMailShellData={loadMailShellData}
        updateAccountSettings={updateAccountSettings}
      />,
    );

    expect(await screen.findByText("Roadmap review")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    const settings = screen.getByRole("dialog", { name: "Settings" });

    fireEvent.change(within(settings).getByLabelText("Account display name"), {
      target: { value: "Sender" },
    });
    fireEvent.click(within(settings).getByLabelText("Include in unified inbox"));
    fireEvent.change(within(settings).getByLabelText("Signature HTML"), {
      target: { value: "<p>Regards</p>" },
    });
    fireEvent.click(within(settings).getByRole("button", { name: "Save settings" }));

    await waitFor(() =>
      expect(updateAccountSettings).toHaveBeenCalledWith({
        accountId: "acct_personal",
        displayName: "Sender",
        localPreferences: { includeInUnifiedInbox: false },
        signatureHtml: "<p>Regards</p>",
      }),
    );
    await waitFor(() => expect(loadMailShellData).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("Account settings were saved.")).toBeVisible();
    expect(screen.getByRole("button", { name: /Sender/ })).toBeVisible();
  });

  it("renders loading and error states for the shell surfaces", () => {
    const loadState: MailShellLoadState = {
      accounts: "loading",
      folders: "error",
      messages: "error",
    };

    render(<App initialLoadState={loadState} />);

    expect(screen.getByText("Loading accounts")).toBeVisible();
    expect(screen.getByText("Folder list unavailable")).toBeVisible();
    expect(screen.getByRole("region", { name: "Message error" })).toBeVisible();
    expect(screen.getByRole("region", { name: "Reader error" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Compose" })).toBeDisabled();
    expect(screen.getByRole("searchbox")).toBeDisabled();
  });

  it("retries a failed backend cache load and replaces the error state with fresh mail", async () => {
    const loadMailShellData = vi
      .fn()
      .mockRejectedValueOnce(new Error("cache unavailable"))
      .mockResolvedValueOnce(fixtureMail);

    render(<App loadMailShellData={loadMailShellData} />);

    expect(await screen.findByRole("region", { name: "Message error" })).toBeVisible();
    expect(loadMailShellData).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getAllByRole("button", { name: "Retry" })[0]);

    expect(await screen.findByText("Roadmap review")).toBeVisible();
    expect(loadMailShellData).toHaveBeenCalledTimes(2);
    expect(
      screen.queryByRole("region", { name: "Message error" }),
    ).not.toBeInTheDocument();
  });

  it("opens accountless settings without exposing fake signature data", () => {
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    const settings = screen.getByRole("region", { name: "Settings" });

    expect(settings).toBeVisible();
    expect(screen.getByRole("heading", { name: "Signatures" })).toBeVisible();
    expect(within(settings).getByText("No accounts configured.")).toBeVisible();
    expect(screen.queryByText("Personal signature")).not.toBeInTheDocument();
  });
});
