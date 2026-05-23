import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { vi } from "vitest";
import App from "./App";
import type {
  MailShellData,
  MailShellLoadState,
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
      id: "folder_work_inbox",
      accountId: "acct_work",
      name: "Inbox",
      kind: "inbox",
      unreadCount: 1,
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
      receivedAt: "09:30",
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
      receivedAt: "Yesterday",
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
      receivedAt: "08:15",
      unread: true,
      flagged: true,
      bodyText: "Can you confirm the invoice number?",
      bodyHtml:
        '<p>Can you <strong>confirm</strong> the invoice number?</p><img src="https://tracker.example/pixel.png" alt="tracking pixel"><script>alert("xss")</script>',
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
  ],
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
        receivedAt: `${Math.floor(index / 60)
          .toString()
          .padStart(2, "0")}:${(index % 60).toString().padStart(2, "0")}`,
        unread: index % 2 === 0,
        flagged: index % 10 === 0 || index === messageCount - 1,
        bodyText: `Body for bulk message ${paddedIndex}.`,
        bodyHtml: null,
        attachments: [],
      };
    }),
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

  it("renders selected message detail through safe body and attachment boundaries", () => {
    const { container } = render(<App initialData={fixtureMail} />);

    fireEvent.click(screen.getByRole("button", { name: /Invoice question/ }));

    const reader = screen.getByRole("region", { name: "Selected message" });
    expect(
      within(reader).getByRole("heading", { name: "Invoice question" }),
    ).toBeVisible();
    expect(reader).toHaveTextContent("Can you confirm the invoice number?");
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
    expect(reader).not.toHaveTextContent("AppData");
    expect(reader).not.toHaveTextContent("att_invoice.pdf");
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
    expect(firstMessage).toHaveTextContent("00:00");
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
    expect(lastMessage).toHaveTextContent("166:39");
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
