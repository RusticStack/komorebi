import React from "react";
import ReactDOM from "react-dom/client";
import App from "../App";
import { matchSupportedLanguage } from "../i18n";
import type {
  AttachmentCommandRequest,
  AttachmentDownloadResponse,
  AttachmentPreviewResponse,
  AttachmentSaveResponse,
  ComposerAttachment,
  ComposerForwardedAttachmentStageRequest,
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
  MailMessagePageRequest,
  MailMessagePageResponse,
  MailSearchRequest,
  MailSearchResponse,
  MailShellData,
  RecentRecipientSearchRequest,
  RecentRecipientSuggestion,
  RemoteContentPreferenceRequest,
  RemoteContentPreferenceResponse,
  UiPreferencesView,
} from "../mail/shell/mailShellState";
import {
  createPerformanceMailFixture,
  normalizePerformanceMessageCount,
  searchPerformanceMailFixture,
  type PerformanceFixtureMetadata,
} from "../perf/mailPerformanceFixtures";
import {
  messageMatchesClientMailSearch,
  parseClientMailSearchQuery,
} from "../mail/search/searchQuery";

type E2eEvent =
  | { name: "load-mail-shell" }
  | { name: "load-message-page"; request: MailMessagePageRequest }
  | { name: "list-drafts" }
  | { name: "search-mail"; request: MailSearchRequest }
  | { name: "allow-remote-content"; request: RemoteContentPreferenceRequest }
  | { name: "open-external-link"; request: ExternalLinkOpenRequest }
  | { name: "perform-mail-action"; request: MailActionRequest }
  | { name: "preview-attachment"; request: AttachmentCommandRequest }
  | { name: "save-attachment"; request: AttachmentCommandRequest }
  | { name: "download-attachment"; request: AttachmentCommandRequest }
  | { name: "save-draft"; request: ComposerDraftSaveRequest }
  | { name: "discard-draft"; request: ComposerDraftDiscardRequest }
  | { name: "stage-attachment"; request: ComposerAttachmentStageRequest }
  | { name: "stage-forwarded-attachments"; request: ComposerForwardedAttachmentStageRequest }
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
      id: "folder_work_archive",
      kind: "archive",
      name: "Archive",
      totalCount: 0,
      unreadCount: 0,
    },
    {
      accountId: "acct_work",
      id: "folder_work_trash",
      kind: "trash",
      name: "Trash",
      totalCount: 0,
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
        {
          contentType: "text/plain",
          displayFilename: "notes.txt",
          disposition: "attachment",
          id: "att_invoice_notes",
          sizeBytes: 20,
        },
      ],
      bodyHtml:
        '<p>Can you <strong>confirm</strong> the invoice number in the <a href="https://example.com/invoice">invoice portal</a>?</p><a href="javascript:alert(1)">bad link</a><img src="https://tracker.example/pixel.png" alt="tracking pixel">',
      bodyText: "Can you confirm the invoice number?",
      ccAddresses: [{ address: "ada@example.com", name: "Ada" }],
      flagged: true,
      folderId: "folder_work_inbox",
      id: "msg_invoice",
      inReplyTo: null,
      preview: "Can you confirm the invoice number?",
      receivedAt: "2026-05-22T08:15:00",
      references: ["<thread-root@example.com>"],
      rfcMessageId: "<invoice-question@example.com>",
      sender: "Finance",
      senderEmail: "finance@example.com",
      subject: "Invoice question",
      toAddresses: [{ address: "work@example.com", name: "Work" }],
      unread: true,
    },
  ],
};

const longStringToken =
  "RegionalizationAccessibilityVerificationTokenWithNoNaturalBreaks1234567890";

const longStringFixtureMail: MailShellData = {
  accounts: [
    {
      email: "internationalization-operations-and-quality-review@example.com",
      id: "acct_long",
      label:
        "Internationalization operations account with a deliberately lengthy display name",
      lastSuccessfulSyncAt: "2026-05-25T08:15:00",
      signatureHtml: null,
      status: "reconnect-needed",
      statusMessage:
        "Reauthentication is required before the next sync cycle can continue for this long localized layout fixture.",
      statusUpdatedAt: "2026-05-24T17:40:00",
      unreadCount: 12345,
    },
  ],
  folders: [
    {
      accountId: "acct_long",
      id: "folder_long_inbox",
      kind: "inbox",
      name: `Inbox for localization screenshots ${longStringToken}`,
      totalCount: 1,
      unreadCount: 1,
    },
    {
      accountId: "acct_long",
      id: "folder_long_archive",
      kind: "archive",
      name: `Archive with extra long translated folder label ${longStringToken}`,
      totalCount: 0,
      unreadCount: 0,
    },
    {
      accountId: "acct_long",
      id: "folder_long_trash",
      kind: "trash",
      name: `Trash folder with narrow layout stress wording ${longStringToken}`,
      totalCount: 0,
      unreadCount: 0,
    },
  ],
  messages: [
    {
      accountId: "acct_long",
      attachments: [
        {
          contentType: "text/plain",
          displayFilename: `quarterly-regionalization-accessibility-notes-${longStringToken}.txt`,
          disposition: "attachment",
          id: "att_long_text",
          sizeBytes: 1536,
        },
      ],
      bodyHtml: null,
      bodyText:
        "This message exists only to verify that translated labels, relative dates, long subjects, long senders, attachment rows, and reader copy fit without horizontal overflow.",
      flagged: true,
      folderId: "folder_long_inbox",
      id: "msg_long_strings",
      preview:
        "Long preview text checks line clamping, wrapping, and translated status surfaces without using mock production data.",
      receivedAt: "2026-05-24T09:30:00",
      sender:
        "Localization Quality Assurance Distribution Group With A Long Sender Name",
      senderEmail: "localization-quality-assurance@example.com",
      subject: `Regionalization layout verification ${longStringToken}`,
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
const fixtureKind = urlParams.get("fixture");
const requestedLanguage =
  matchSupportedLanguage(urlParams.get("language")) ?? "en";
const performanceFixture =
  fixtureKind === "performance"
    ? createPerformanceMailFixture({
        messageCount: normalizePerformanceMessageCount(
          Number(urlParams.get("messages")),
        ),
      })
    : null;
const activeMail =
  performanceFixture?.data ??
  (fixtureKind === "long-strings" ? longStringFixtureMail : fixtureMail);
window.__KOMOREBI_PERF_FIXTURE__ = performanceFixture?.metadata;

async function loadUiPreferences(): Promise<UiPreferencesView> {
  return { language: requestedLanguage };
}

async function loadMailShellData(): Promise<MailShellData> {
  e2eEvents.push({ name: "load-mail-shell" });
  await delay(performanceFixture ? 0 : 50);

  return clone(activeMail);
}

async function loadMessagePage(
  request: MailMessagePageRequest,
  signal: AbortSignal,
): Promise<MailMessagePageResponse> {
  e2eEvents.push({ name: "load-message-page", request });
  await delay(25, signal);

  const limit = Math.max(1, Math.min(request.limit ?? 100, 250));
  const messages = activeMail.messages
    .filter((message) => {
      if (request.folderId) {
        return (
          message.folderId === request.folderId &&
          (!request.accountId || message.accountId === request.accountId)
        );
      }

      if (request.unifiedInbox) {
        const folder = activeMail.folders.find(
          (candidate) =>
            candidate.id === message.folderId &&
            candidate.accountId === message.accountId,
        );

        return folder?.kind === "inbox";
      }

      return !request.accountId || message.accountId === request.accountId;
    })
    .sort((left, right) => {
      const byReceivedAt = right.receivedAt.localeCompare(left.receivedAt);
      return byReceivedAt === 0 ? left.id.localeCompare(right.id) : byReceivedAt;
    });

  const startIndex = request.cursor
    ? messages.findIndex((message) => message.id === request.cursor?.messageId) + 1
    : 0;
  const boundedStartIndex = Math.max(0, startIndex);
  const page = messages.slice(boundedStartIndex, boundedStartIndex + limit);
  const hasMore = boundedStartIndex + page.length < messages.length;
  const lastPageMessage = page[page.length - 1] ?? null;

  return {
    hasMore,
    messages: clone(page),
    nextCursor: hasMore
      ? lastPageMessage?.pageCursor ?? {
          messageId: lastPageMessage?.id ?? "",
          sortKey: lastPageMessage?.receivedAt ?? "",
        }
      : null,
    totalCount: messages.length,
  };
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
  const parsed = parseClientMailSearchQuery(request.query);
  const archiveMatches =
    query === "phoenix" ||
    messageMatchesClientMailSearch(archiveSearchResult, parsed);
  const results = [
    ...(archiveMatches ? [archiveSearchResult] : []),
    ...activeMail.messages.filter((message) =>
      messageMatchesClientMailSearch(message, parsed),
    ),
  ];

  return {
    highlightTerms: parsed.highlightTerms,
    indexStatus: "ready",
    parsedQuery: parsed.parsedQuery,
    query: request.query,
    results: clone(results),
  };
}

async function allowRemoteContent(
  request: RemoteContentPreferenceRequest,
): Promise<RemoteContentPreferenceResponse> {
  e2eEvents.push({ name: "allow-remote-content", request });
  await delay(20);

  const message = activeMail.messages.find(
    (candidate) =>
      candidate.accountId === request.accountId &&
      candidate.folderId === request.folderId &&
      candidate.id === request.messageId,
  );
  if (!message) {
    throw new Error("message was not found");
  }

  message.remoteContentAllowed = true;
  message.remoteContentAllowedScope = request.scope;

  return {
    accountId: request.accountId,
    messageId: request.messageId,
    remoteContentAllowed: true,
    scope: request.scope,
    senderEmail: request.scope === "sender" ? message.senderEmail : null,
  };
}

async function openExternalLink(
  request: ExternalLinkOpenRequest,
): Promise<ExternalLinkOpenResponse> {
  e2eEvents.push({ name: "open-external-link", request });
  await delay(20);

  return {
    opened: true,
    openedUrl: request.url,
  };
}

async function performMailAction(
  request: MailActionRequest,
): Promise<MailActionResponse> {
  e2eEvents.push({ name: "perform-mail-action", request });
  await delay(20);

  const message = request.messageId
    ? activeMail.messages.find(
        (candidate) =>
          candidate.accountId === request.accountId &&
          candidate.folderId === request.folderId &&
          candidate.id === request.messageId,
      )
    : null;

  if (request.action.kind === "mark-read" && message) {
    message.unread = !request.action.read;
    return {
      accountId: request.accountId,
      action: request.action,
      mutation: { kind: "flags-updated", read: request.action.read },
      retry: { kind: "retryable", maxAttempts: 3 },
      undo: {
        accountId: request.accountId,
        action: { kind: "mark-read", read: !request.action.read },
        folderId: request.folderId,
        messageId: request.messageId ?? message.id,
      },
    };
  }

  if (request.action.kind === "expunge-deleted") {
    return {
      accountId: request.accountId,
      action: request.action,
      mutation: {
        kind: "expunged",
        folderId: request.folderId,
        removedCount: 0,
      },
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
  if (message) {
    message.folderId = targetFolderId;
  }

  return {
    accountId: request.accountId,
    action: request.action,
    mutation: {
      kind: "moved",
      fromFolderId: request.folderId,
      toFolderId: targetFolderId,
    },
    retry: { kind: "retryable", maxAttempts: 3 },
    undo: request.messageId
      ? {
          accountId: request.accountId,
          action: { kind: "move", targetFolderId: request.folderId },
          folderId: targetFolderId,
          messageId: request.messageId,
        }
      : null,
  };
}

async function previewAttachment(
  request: AttachmentCommandRequest,
): Promise<AttachmentPreviewResponse> {
  e2eEvents.push({ name: "preview-attachment", request });
  await delay(20);

  return {
    attachmentId: request.attachmentId,
    autoOpen: false,
    bytes: null,
    contentType: "text/plain",
    displayFilename: "notes.txt",
    kind: "plain-text",
    sizeBytes: 20,
    text: "E2E attachment notes",
  };
}

async function saveAttachment(
  request: AttachmentCommandRequest,
): Promise<AttachmentSaveResponse> {
  e2eEvents.push({ name: "save-attachment", request });
  await delay(20);

  return {
    attachmentId: request.attachmentId,
    bytesWritten: 20,
    contentType: "text/plain",
    destinationLabel: "notes.txt",
    displayFilename: "notes.txt",
    savedFilename: "notes.txt",
    sizeBytes: 20,
  };
}

async function downloadAttachment(
  request: AttachmentCommandRequest,
): Promise<AttachmentDownloadResponse> {
  e2eEvents.push({ name: "download-attachment", request });
  await delay(20);

  return {
    attachmentId: request.attachmentId,
    bytes: Array.from(new TextEncoder().encode("E2E attachment notes")),
    contentType: "text/plain",
    displayFilename: "notes.txt",
    sizeBytes: 20,
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

async function stageForwardedAttachments(
  request: ComposerForwardedAttachmentStageRequest,
): Promise<ComposerAttachment[]> {
  e2eEvents.push({ name: "stage-forwarded-attachments", request });
  await delay(20);

  return request.sourceAttachmentIds.map((attachmentId) => ({
    accountId: request.accountId,
    contentId: null,
    contentType: attachmentId.endsWith("notes") ? "text/plain" : "application/pdf",
    displayFilename: attachmentId.endsWith("notes") ? "notes.txt" : "invoice.pdf",
    disposition: "attachment",
    draftId: request.draftId,
    id: `catt_forwarded_${attachmentId}`,
    sizeBytes: attachmentId.endsWith("notes") ? 20 : 24_576,
  }));
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
      allowRemoteContent={allowRemoteContent}
      discardComposerDraft={discardComposerDraft}
      downloadAttachment={downloadAttachment}
      loadComposerDrafts={loadComposerDrafts}
      loadMailShellData={loadMailShellData}
      loadMessagePage={performanceFixture ? undefined : loadMessagePage}
      loadRecentRecipients={listRecentRecipients}
      loadUiPreferences={loadUiPreferences}
      openExternalLink={openExternalLink}
      performMailAction={performMailAction}
      previewAttachment={previewAttachment}
      saveAttachment={saveAttachment}
      saveComposerDraft={saveComposerDraft}
      searchMail={searchMail}
      searchDebounceMs={performanceFixture ? 25 : undefined}
      sendComposerMail={sendComposerMail}
      stageComposerAttachment={stageComposerAttachment}
      stageForwardedAttachments={stageForwardedAttachments}
    />
  </React.StrictMode>,
);
