import { fireEvent, render, screen, within } from "@testing-library/react";
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
      bodyHtml: "<p>Can you confirm the invoice number?</p>",
    },
  ],
};

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
