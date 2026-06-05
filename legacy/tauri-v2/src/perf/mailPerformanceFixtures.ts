import type {
  MailAttachment,
  MailMessage,
  MailSearchRequest,
  MailSearchResponse,
  MailShellData,
} from "../mail/shell/mailShellState";
import {
  messageMatchesClientMailSearch,
  parseClientMailSearchQuery,
} from "../mail/search/searchQuery";

export const PERFORMANCE_FIXTURE_MESSAGE_COUNT = 10_000;
export const PERFORMANCE_FIXTURE_MAX_MESSAGE_COUNT = 50_000;
export const PERFORMANCE_FIXTURE_SEED = "komorebi-perf-fixture-v1";

const ACCOUNT_IDS = ["acct_perf_personal", "acct_perf_work"] as const;
const INBOX_FOLDER_IDS = ["folder_perf_personal_inbox", "folder_perf_work_inbox"];
const REFERENCE_TIME_UTC = Date.UTC(2026, 4, 23, 12, 0, 0);
const ATTACHMENT_EVERY_N_MESSAGES = 20;
const NEEDLE_MESSAGE_NUMBER = 9_999;

export type PerformanceFixtureMetadata = {
  seed: string;
  messageCount: number;
  attachmentCount: number;
  inlineAttachmentCount: number;
  totalAttachmentBytes: number;
  searchNeedle: string;
  advancedSearchQuery: string;
};

export type PerformanceMailFixture = {
  data: MailShellData;
  metadata: PerformanceFixtureMetadata;
};

export function createPerformanceMailFixture({
  messageCount = PERFORMANCE_FIXTURE_MESSAGE_COUNT,
}: {
  messageCount?: number;
} = {}): PerformanceMailFixture {
  const boundedMessageCount = normalizePerformanceMessageCount(messageCount);
  const messages: MailMessage[] = [];
  let attachmentCount = 0;
  let inlineAttachmentCount = 0;
  let totalAttachmentBytes = 0;

  for (let index = 0; index < boundedMessageCount; index += 1) {
    const messageNumber = index + 1;
    const accountIndex = index % ACCOUNT_IDS.length;
    const attachments = attachmentsForMessage(messageNumber, ACCOUNT_IDS[accountIndex]);
    attachmentCount += attachments.length;
    inlineAttachmentCount += attachments.filter(
      (attachment) => attachment.disposition === "inline",
    ).length;
    totalAttachmentBytes += attachments.reduce(
      (total, attachment) => total + attachment.sizeBytes,
      0,
    );

    messages.push({
      accountId: ACCOUNT_IDS[accountIndex],
      attachments,
      bodyHtml:
        messageNumber % 25 === 0
          ? `<p>Performance body ${messageNumber}</p><p><strong>Rendered HTML branch</strong> with local-only fixture content.</p>`
          : null,
      bodyText: bodyTextForMessage(messageNumber),
      flagged: messageNumber % 17 === 0,
      folderId: INBOX_FOLDER_IDS[accountIndex],
      id: `msg_perf_${messageNumber.toString().padStart(5, "0")}`,
      preview: previewForMessage(messageNumber),
      receivedAt: new Date(REFERENCE_TIME_UTC - index * 60_000).toISOString(),
      sender: senderForMessage(messageNumber),
      senderEmail: `sender${messageNumber}@perf.example`,
      subject: subjectForMessage(messageNumber),
      unread: messageNumber % 3 === 0,
    });
  }

  return {
    data: {
      accounts: [
        {
          email: "personal-perf@example.com",
          id: ACCOUNT_IDS[0],
          label: "Personal Perf",
          signatureHtml: "<p>Personal performance signature</p>",
          status: "connected",
          unreadCount: unreadCountForAccount(messages, ACCOUNT_IDS[0]),
        },
        {
          email: "work-perf@example.com",
          id: ACCOUNT_IDS[1],
          label: "Work Perf",
          signatureHtml: "<p>Work performance signature</p>",
          status: "syncing",
          unreadCount: unreadCountForAccount(messages, ACCOUNT_IDS[1]),
        },
      ],
      folders: [
        {
          accountId: ACCOUNT_IDS[0],
          id: INBOX_FOLDER_IDS[0],
          kind: "inbox",
          name: "Inbox",
          totalCount: countForAccount(messages, ACCOUNT_IDS[0]),
          unreadCount: unreadCountForAccount(messages, ACCOUNT_IDS[0]),
        },
        {
          accountId: ACCOUNT_IDS[1],
          id: INBOX_FOLDER_IDS[1],
          kind: "inbox",
          name: "Inbox",
          totalCount: countForAccount(messages, ACCOUNT_IDS[1]),
          unreadCount: unreadCountForAccount(messages, ACCOUNT_IDS[1]),
        },
      ],
      messages,
    },
    metadata: {
      attachmentCount,
      advancedSearchQuery: performanceAdvancedSearchQuery(),
      inlineAttachmentCount,
      messageCount: boundedMessageCount,
      searchNeedle: performanceSearchNeedle(),
      seed: PERFORMANCE_FIXTURE_SEED,
      totalAttachmentBytes,
    },
  };
}

export function performanceSearchNeedle() {
  return `needle-${NEEDLE_MESSAGE_NUMBER}`;
}

export function performanceAdvancedSearchQuery() {
  return `subject:${performanceSearchNeedle()}`;
}

export function searchPerformanceMailFixture(
  data: MailShellData,
  request: MailSearchRequest,
): MailSearchResponse {
  const query = request.query.trim();
  const parsed = parseClientMailSearchQuery(query);
  const limit = Math.max(1, Math.min(request.limit ?? 100, 500));

  if (!query) {
    return {
      highlightTerms: [],
      indexStatus: "ready",
      parsedQuery: parsed.parsedQuery,
      query: request.query,
      results: [],
    };
  }

  const scopedFolderIds = new Set(
    data.folders
      .filter((folder) => {
        if (request.accountId && folder.accountId !== request.accountId) {
          return false;
        }

        if (request.folderId && folder.id !== request.folderId) {
          return false;
        }

        return true;
      })
      .map((folder) => folder.id),
  );

  const results: MailMessage[] = [];
  for (const message of data.messages) {
    if (!scopedFolderIds.has(message.folderId)) {
      continue;
    }

    if (messageMatchesClientMailSearch(message, parsed)) {
      results.push(message);
      if (results.length >= limit) {
        break;
      }
    }
  }

  return {
    highlightTerms: parsed.highlightTerms,
    indexStatus: "ready",
    parsedQuery: parsed.parsedQuery,
    query: request.query,
    results,
  };
}

export function normalizePerformanceMessageCount(messageCount: number) {
  if (!Number.isFinite(messageCount)) {
    return PERFORMANCE_FIXTURE_MESSAGE_COUNT;
  }

  return Math.min(
    Math.max(Math.trunc(messageCount), PERFORMANCE_FIXTURE_MESSAGE_COUNT),
    PERFORMANCE_FIXTURE_MAX_MESSAGE_COUNT,
  );
}

function subjectForMessage(messageNumber: number) {
  if (messageNumber === NEEDLE_MESSAGE_NUMBER) {
    return `Performance message ${messageNumber} ${performanceSearchNeedle()}`;
  }

  return `Performance message ${messageNumber}`;
}

function previewForMessage(messageNumber: number) {
  if (messageNumber % ATTACHMENT_EVERY_N_MESSAGES === 0) {
    return `Attachment-heavy performance row ${messageNumber}`;
  }

  return `Local cache performance preview ${messageNumber}`;
}

function bodyTextForMessage(messageNumber: number) {
  const attachmentMarker =
    messageNumber % ATTACHMENT_EVERY_N_MESSAGES === 0
      ? ` attachment-heavy-${messageNumber}`
      : "";

  return [
    `Synthetic performance body ${messageNumber}.`,
    "This local-only fixture exercises search, virtualization, and reader rendering without real mailbox data.",
    `Batch bucket ${messageNumber % 100}.${attachmentMarker}`,
  ].join(" ");
}

function senderForMessage(messageNumber: number) {
  const bucket = messageNumber % 12;
  return `Perf Sender ${bucket.toString().padStart(2, "0")}`;
}

function attachmentsForMessage(
  messageNumber: number,
  accountId: (typeof ACCOUNT_IDS)[number],
): MailAttachment[] {
  if (messageNumber % ATTACHMENT_EVERY_N_MESSAGES !== 0) {
    return [];
  }

  const baseId = `att_perf_${messageNumber.toString().padStart(5, "0")}`;
  const attachmentType =
    Math.trunc(messageNumber / ATTACHMENT_EVERY_N_MESSAGES) % 4;
  const common = {
    id: `${baseId}_${attachmentType}`,
    displayFilename: `performance-${messageNumber}`,
  };

  switch (attachmentType) {
    case 0:
      return [
        {
          ...common,
          contentType: "application/pdf",
          displayFilename: `${common.displayFilename}.pdf`,
          disposition: "attachment",
          sizeBytes: 196_608,
        },
      ];
    case 1:
      return [
        {
          ...common,
          contentType: "image/png",
          displayFilename: `${common.displayFilename}.png`,
          disposition: "inline",
          sizeBytes: 65_536,
        },
      ];
    case 2:
      return [
        {
          ...common,
          contentType: "text/plain",
          displayFilename: `${common.displayFilename}.txt`,
          disposition: "attachment",
          sizeBytes: 12_288,
        },
      ];
    default:
      return [
        {
          ...common,
          contentType: "application/zip",
          displayFilename: `${common.displayFilename}-${accountId}.zip`,
          disposition: "attachment",
          sizeBytes: 1_572_864,
        },
      ];
  }
}

function countForAccount(messages: MailMessage[], accountId: string) {
  return messages.filter((message) => message.accountId === accountId).length;
}

function unreadCountForAccount(messages: MailMessage[], accountId: string) {
  return messages.filter(
    (message) => message.accountId === accountId && message.unread,
  ).length;
}
