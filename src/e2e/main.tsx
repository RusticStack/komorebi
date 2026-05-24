import React from "react";
import ReactDOM from "react-dom/client";
import App from "../App";
import "../i18n";
import type {
  ComposerAttachment,
  ComposerAttachmentStageRequest,
  ComposerDraft,
  ComposerDraftDiscardRequest,
  ComposerDraftSaveRequest,
  ComposerDraftSaveResponse,
  ComposerSendRequest,
  ComposerSendResponse,
  MailMessage,
  MailSearchRequest,
  MailSearchResponse,
  MailShellData,
  RecentRecipientSearchRequest,
  RecentRecipientSuggestion,
} from "../mail/shell/mailShellState";
import {
  createPerformanceMailFixture,
  normalizePerformanceMessageCount,
  searchPerformanceMailFixture,
  type PerformanceFixtureMetadata,
} from "../perf/mailPerformanceFixtures";

type E2eEvent =
  | { name: "load-mail-shell" }
  | { name: "list-drafts" }
  | { name: "search-mail"; request: MailSearchRequest }
  | { name: "save-draft"; request: ComposerDraftSaveRequest }
  | { name: "discard-draft"; request: ComposerDraftDiscardRequest }
  | { name: "stage-attachment"; request: ComposerAttachmentStageRequest }
  | { name: "send-mail"; request: ComposerSendRequest }
  | { name: "recent-recipients"; request: RecentRecipientSearchRequest };

declare global {
  interface Window {
    __KOMOREBI_E2E_EVENTS__?: E2eEvent[];
    __KOMOREBI_PERF_FIXTURE__?: PerformanceFixtureMetadata;
  }
}

const e2eEvents: E2eEvent[] = [];
window.__KOMOREBI_E2E_EVENTS__ = e2eEvents;

const fixtureMail: MailShellData = {
  accounts: [
    {
      email: "person@example.com",
      id: "acct_personal",
      label: "Personal",
      signatureHtml: "<p>Personal signature</p>",
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
      totalCount: 2,
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
      bodyText: "Review notes for the local cache work.",
      flagged: false,
      folderId: "folder_personal_inbox",
      id: "msg_roadmap",
      preview: "Review notes for the local cache work.",
      receivedAt: "2026-05-22T09:30:00",
      sender: "Ada",
      senderEmail: "ada@example.com",
      subject: "Roadmap review",
      unread: true,
    },
    {
      accountId: "acct_personal",
      attachments: [],
      bodyHtml: null,
      bodyText: "Stored outside the unified inbox.",
      flagged: false,
      folderId: "folder_personal_archive",
      id: "msg_archive",
      preview: "Stored outside the unified inbox.",
      receivedAt: "2026-05-21T09:30:00",
      sender: "Archive Bot",
      senderEmail: "archive@example.com",
      subject: "Archived receipt",
      unread: false,
    },
    {
      accountId: "acct_work",
      attachments: [
        {
          contentType: "application/pdf",
          displayFilename: "invoice.pdf",
          disposition: "attachment",
          id: "att_invoice",
          sizeBytes: 24_576,
        },
      ],
      bodyHtml:
        '<p>Can you <strong>confirm</strong> the invoice number?</p><img src="https://tracker.example/pixel.png" alt="tracking pixel">',
      bodyText: "Can you confirm the invoice number?",
      flagged: true,
      folderId: "folder_work_inbox",
      id: "msg_invoice",
      preview: "Can you confirm the invoice number?",
      receivedAt: "2026-05-22T08:15:00",
      sender: "Finance",
      senderEmail: "finance@example.com",
      subject: "Invoice question",
      unread: true,
    },
  ],
};

const archiveSearchResult: MailMessage = {
  accountId: "acct_personal",
  attachments: [],
  bodyHtml: null,
  bodyText: "Phoenix appears only in the FTS-backed archive result.",
  flagged: false,
  folderId: "folder_personal_archive",
  id: "msg_search_archive",
  preview: "Phoenix appears only in the FTS-backed archive result.",
  receivedAt: "2026-05-20T07:45:00",
  sender: "Research",
  senderEmail: "research@example.com",
  subject: "Phoenix report",
  unread: false,
};

const savedDrafts: ComposerDraft[] = [
  {
    accountId: "acct_personal",
    attachments: [
      {
        accountId: "acct_personal",
        contentId: null,
        contentType: "text/plain",
        displayFilename: "briefing.txt",
        disposition: "attachment",
        draftId: "draft_e2e_saved",
        id: "catt_e2e_briefing",
        sizeBytes: 5,
      },
    ],
    autosaveRevision: 3,
    bcc: [],
    bodyHtml: "<p>Draft body from local cache.</p><p>Personal signature</p>",
    bodyText: "Draft body from local cache.\nPersonal signature",
    cc: [],
    id: "draft_e2e_saved",
    subject: "Saved local draft",
    to: [{ address: "qa@example.com", name: null }],
  },
];

const urlParams = new URLSearchParams(window.location.search);
const performanceFixture =
  urlParams.get("fixture") === "performance"
    ? createPerformanceMailFixture({
        messageCount: normalizePerformanceMessageCount(
          Number(urlParams.get("messages")),
        ),
      })
    : null;
const activeMail = performanceFixture?.data ?? fixtureMail;
window.__KOMOREBI_PERF_FIXTURE__ = performanceFixture?.metadata;

async function loadMailShellData(): Promise<MailShellData> {
  e2eEvents.push({ name: "load-mail-shell" });
  await delay(performanceFixture ? 0 : 50);

  return clone(activeMail);
}

async function loadComposerDrafts(): Promise<ComposerDraft[]> {
  e2eEvents.push({ name: "list-drafts" });
  await delay(performanceFixture ? 0 : 30);

  return clone(savedDrafts);
}

async function searchMail(
  request: MailSearchRequest,
  signal: AbortSignal,
): Promise<MailSearchResponse> {
  e2eEvents.push({ name: "search-mail", request });
  await delay(performanceFixture ? 0 : 75, signal);

  if (performanceFixture) {
    return clone(searchPerformanceMailFixture(activeMail, request));
  }

  const query = request.query.toLocaleLowerCase();
  const results =
    query === "phoenix"
      ? [archiveSearchResult]
      : activeMail.messages.filter((message) =>
          [
            message.sender,
            message.senderEmail,
            message.subject,
            message.preview,
            message.bodyText ?? "",
          ].some((value) => value.toLocaleLowerCase().includes(query)),
        );

  return {
    indexStatus: "ready",
    query: request.query,
    results: clone(results),
  };
}

async function saveComposerDraft(
  request: ComposerDraftSaveRequest,
): Promise<ComposerDraftSaveResponse> {
  e2eEvents.push({ name: "save-draft", request });
  await delay(20);

  return {
    accountId: request.accountId,
    autosaveRevision: e2eEvents.length,
    id: request.draftId,
    state: "active",
  };
}

async function discardComposerDraft(
  request: ComposerDraftDiscardRequest,
): Promise<ComposerDraftSaveResponse> {
  e2eEvents.push({ name: "discard-draft", request });
  await delay(20);

  return {
    accountId: request.accountId,
    autosaveRevision: e2eEvents.length,
    id: request.draftId,
    state: "discarded",
  };
}

async function stageComposerAttachment(
  request: ComposerAttachmentStageRequest,
): Promise<ComposerAttachment> {
  e2eEvents.push({ name: "stage-attachment", request });
  await delay(20);

  return {
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
  };
}

async function sendComposerMail(
  request: ComposerSendRequest,
): Promise<ComposerSendResponse> {
  e2eEvents.push({ name: "send-mail", request });
  await delay(50);

  return {
    accountId: request.accountId,
    attachmentBytesStreamed: request.attachmentIds.length,
    draftId: request.draftId,
    messageSizeBytes: 512,
    progress: [
      "account-resolved",
      "draft-prepared",
      "auth-resolved",
      "mime-composed",
      "smtp-accepted",
      "sent-state-recorded",
      "draft-finalized",
    ],
    providerResponse: "250 accepted",
    rfcMessageId: "<e2e-smoke@example.com>",
    sentFolderPolicy: "local_receipt_only",
    sentFolderSyncState: "unsupported",
  };
}

async function listRecentRecipients(
  request: RecentRecipientSearchRequest,
  signal: AbortSignal,
): Promise<RecentRecipientSuggestion[]> {
  e2eEvents.push({ name: "recent-recipients", request });
  await delay(50, signal);

  return [
    {
      accountId: request.accountId,
      address: "qa@example.com",
      id: "rr_qa",
      lastUsedAt: "2026-05-23T10:00:00Z",
      name: "QA Recipient",
      useCount: 2,
    },
  ].filter((recipient) =>
    `${recipient.name ?? ""} ${recipient.address}`
      .toLocaleLowerCase()
      .includes(request.query.toLocaleLowerCase()),
  );
}

function delay(ms: number, signal?: AbortSignal) {
  if (signal?.aborted) {
    return Promise.reject(new DOMException("E2E operation cancelled.", "AbortError"));
  }

  return new Promise<void>((resolve, reject) => {
    const timeout = window.setTimeout(resolve, ms);
    signal?.addEventListener(
      "abort",
      () => {
        window.clearTimeout(timeout);
        reject(new DOMException("E2E operation cancelled.", "AbortError"));
      },
      { once: true },
    );
  });
}

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App
      discardComposerDraft={discardComposerDraft}
      loadComposerDrafts={loadComposerDrafts}
      loadMailShellData={loadMailShellData}
      loadRecentRecipients={listRecentRecipients}
      saveComposerDraft={saveComposerDraft}
      searchMail={searchMail}
      searchDebounceMs={performanceFixture ? 25 : undefined}
      sendComposerMail={sendComposerMail}
      stageComposerAttachment={stageComposerAttachment}
    />
  </React.StrictMode>,
);
