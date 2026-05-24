import type {
  MailMessage,
  MailSearchParsedFilter,
  MailSearchParsedQuery,
} from "../shell/mailShellState";

export type ClientMailSearchPlan = {
  parsedQuery: MailSearchParsedQuery;
  highlightTerms: string[];
  senderFilter: string | null;
  subjectFilter: string | null;
  bodyFilter: string | null;
  hasAttachmentFilter: boolean | null;
  readFilter: boolean | null;
  flaggedFilter: boolean | null;
};

type SearchToken =
  | { kind: "text"; value: string }
  | { kind: "filter"; filterKind: string; value: string };

const MAX_SEARCH_TERMS = 12;

export function parseClientMailSearchQuery(query: string): ClientMailSearchPlan {
  const tokens = tokenize(query);
  const textTerms: string[] = [];
  const highlightTerms: string[] = [];
  const filters: MailSearchParsedFilter[] = [];
  let senderFilter: string | null = null;
  let subjectFilter: string | null = null;
  let bodyFilter: string | null = null;
  let hasAttachmentFilter: boolean | null = null;
  let readFilter: boolean | null = null;
  let flaggedFilter: boolean | null = null;

  for (const token of tokens) {
    if (token.kind === "text") {
      const terms = termsFromText(token.value);
      pushTerms(textTerms, terms);
      pushTerms(highlightTerms, terms);
      continue;
    }

    const value = normalizeValue(token.value);
    if (!value) {
      continue;
    }

    switch (token.filterKind) {
      case "from":
        senderFilter = value.toLocaleLowerCase();
        filters.push({ kind: "from", value });
        pushTerms(highlightTerms, termsFromText(value));
        break;
      case "subject":
        subjectFilter = value.toLocaleLowerCase();
        filters.push({ kind: "subject", value });
        pushTerms(highlightTerms, termsFromText(value));
        break;
      case "body":
        bodyFilter = value.toLocaleLowerCase();
        filters.push({ kind: "body", value });
        pushTerms(highlightTerms, termsFromText(value));
        break;
      case "has":
        if (["attachment", "attachments"].includes(value.toLocaleLowerCase())) {
          hasAttachmentFilter = true;
          filters.push({ kind: "has", value: "attachment" });
        }
        break;
      case "is":
        switch (value.toLocaleLowerCase()) {
          case "read":
            readFilter = true;
            filters.push({ kind: "is", value: "read" });
            break;
          case "unread":
            readFilter = false;
            filters.push({ kind: "is", value: "unread" });
            break;
          case "flagged":
          case "starred":
            flaggedFilter = true;
            filters.push({ kind: "is", value: "flagged" });
            break;
        }
        break;
    }
  }

  return {
    bodyFilter,
    flaggedFilter,
    hasAttachmentFilter,
    highlightTerms,
    parsedQuery: {
      filters,
      textTerms,
    },
    readFilter,
    senderFilter,
    subjectFilter,
  };
}

export function messageMatchesClientMailSearch(
  message: MailMessage,
  plan: ClientMailSearchPlan,
) {
  if (
    plan.senderFilter &&
    ![message.sender, message.senderEmail].some((value) =>
      value.toLocaleLowerCase().includes(plan.senderFilter ?? ""),
    )
  ) {
    return false;
  }

  if (
    plan.subjectFilter &&
    !message.subject.toLocaleLowerCase().includes(plan.subjectFilter)
  ) {
    return false;
  }

  if (
    plan.bodyFilter &&
    ![message.bodyText ?? "", message.bodyHtml ?? "", message.preview].some(
      (value) => value.toLocaleLowerCase().includes(plan.bodyFilter ?? ""),
    )
  ) {
    return false;
  }

  if (
    plan.hasAttachmentFilter !== null &&
    (message.attachments.length > 0) !== plan.hasAttachmentFilter
  ) {
    return false;
  }

  if (plan.readFilter !== null && message.unread === plan.readFilter) {
    return false;
  }

  if (plan.flaggedFilter !== null && message.flagged !== plan.flaggedFilter) {
    return false;
  }

  return plan.parsedQuery.textTerms.every((term) =>
    [
      message.sender,
      message.senderEmail,
      message.subject,
      message.preview,
      message.bodyText ?? "",
      message.bodyHtml ?? "",
    ].some((value) => value.toLocaleLowerCase().includes(term)),
  );
}

function tokenize(query: string): SearchToken[] {
  const tokens: SearchToken[] = [];
  let index = 0;

  while (index < query.length) {
    while (index < query.length && /\s/.test(query[index] ?? "")) {
      index += 1;
    }
    if (index >= query.length) {
      break;
    }

    if (query[index] === '"') {
      const quoted = readQuotedValue(query, index + 1);
      tokens.push({ kind: "text", value: quoted.value });
      index = quoted.nextIndex;
      continue;
    }

    const tokenStart = index;
    while (
      index < query.length &&
      !/\s/.test(query[index] ?? "") &&
      query[index] !== ":"
    ) {
      index += 1;
    }

    if (query[index] === ":") {
      const rawKey = query.slice(tokenStart, index).toLocaleLowerCase();
      const filterKind = rawKey === "sender" ? "from" : rawKey;
      if (isRecognizedFilter(filterKind)) {
        index += 1;
        if (query[index] === '"') {
          const quoted = readQuotedValue(query, index + 1);
          tokens.push({ filterKind, kind: "filter", value: quoted.value });
          index = quoted.nextIndex;
          continue;
        }

        const valueStart = index;
        while (index < query.length && !/\s/.test(query[index] ?? "")) {
          index += 1;
        }
        tokens.push({
          filterKind,
          kind: "filter",
          value: query.slice(valueStart, index),
        });
        continue;
      }

      while (index < query.length && !/\s/.test(query[index] ?? "")) {
        index += 1;
      }
      tokens.push({ kind: "text", value: query.slice(tokenStart, index) });
      continue;
    }

    tokens.push({ kind: "text", value: query.slice(tokenStart, index) });
  }

  return tokens;
}

function readQuotedValue(query: string, startIndex: number) {
  const endIndex = query.indexOf('"', startIndex);
  if (endIndex === -1) {
    return {
      nextIndex: query.length,
      value: query.slice(startIndex),
    };
  }

  return {
    nextIndex: endIndex + 1,
    value: query.slice(startIndex, endIndex),
  };
}

function isRecognizedFilter(value: string) {
  return ["from", "subject", "body", "has", "is"].includes(value);
}

function normalizeValue(value: string) {
  return value.trim().replace(/\s+/g, " ");
}

function termsFromText(value: string) {
  return value
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean)
    .map((term) => term.toLocaleLowerCase());
}

function pushTerms(target: string[], terms: string[]) {
  for (const term of terms) {
    if (target.length >= MAX_SEARCH_TERMS) {
      return;
    }
    if (!target.includes(term)) {
      target.push(term);
    }
  }
}
