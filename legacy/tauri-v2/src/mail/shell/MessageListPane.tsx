import { useEffect, useRef, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  useVirtualizer,
  type Rect,
  type Virtualizer,
} from "@tanstack/react-virtual";
import { useLocaleFormatters } from "../../i18n";
import type {
  MailMessage,
  MailMessagePageCursor,
  MailShellState,
} from "./mailShellState";
import {
  clampIndex,
  isActivationKey,
  nextIndexForKey,
} from "./keyboardNavigation";
import { messageSenderLabel } from "./mailDisplay";
import { HighlightedText, type MailSearchUiStatus } from "./MailSearchTools";

const MESSAGE_ROW_HEIGHT_PX = 136;
const MESSAGE_LIST_OVERSCAN = 8;
const MESSAGE_PAGE_LOAD_AHEAD_ROWS = 20;

const MESSAGE_LIST_FALLBACK_RECT: Rect = {
  height: 600,
  width: 400,
};

export type MailMessagePageUiStatus = "idle" | "loading" | "error";

export type MailMessagePageUiState = {
  cursor: MailMessagePageCursor | null;
  hasMore: boolean;
  scopeKey: string;
  status: MailMessagePageUiStatus;
  totalCount: number | null;
};

export function MessageListPane({
  accountById,
  folderById,
  isBackendSearchActive,
  paginationStatus,
  state,
  searchStatus,
  visibleMessages,
  onLoadMoreMessages,
  onRetry,
  onRetryPage,
  onRetrySearch,
  searchHighlightTerms,
  onSelectMessage,
  onReadMessage,
}: {
  accountById: Map<string, { label: string }>;
  folderById: Map<string, { name: string }>;
  isBackendSearchActive: boolean;
  paginationStatus: MailMessagePageUiState;
  state: MailShellState;
  searchStatus: MailSearchUiStatus;
  visibleMessages: MailMessage[];
  onLoadMoreMessages?: () => void;
  onRetry?: () => void;
  onRetryPage: () => void;
  onRetrySearch: () => void;
  searchHighlightTerms: string[];
  onSelectMessage: (messageId: string) => void;
  onReadMessage: (messageId: string) => void;
}) {
  const { t } = useTranslation();
  const listRef = useRef<HTMLDivElement | null>(null);
  const messageButtonRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const pendingFocusIndexRef = useRef<number | null>(null);
  // eslint-disable-next-line react-hooks/incompatible-library -- TanStack Virtual owns scroll math locally; the virtualizer is not passed to memoized children.
  const rowVirtualizer = useVirtualizer<HTMLDivElement, HTMLButtonElement>({
    count: visibleMessages.length,
    estimateSize: () => MESSAGE_ROW_HEIGHT_PX,
    getItemKey: (index) => visibleMessages[index]?.id ?? index,
    getScrollElement: () => listRef.current,
    initialRect: MESSAGE_LIST_FALLBACK_RECT,
    observeElementRect: observeMessageListRect,
    overscan: MESSAGE_LIST_OVERSCAN,
    scrollToFn: scrollMessageListToOffset,
  });
  const virtualRows = rowVirtualizer.getVirtualItems();

  function focusMessageAtIndex(messageIndex: number) {
    const boundedIndex = clampIndex(messageIndex, visibleMessages.length);
    const message = visibleMessages[boundedIndex];
    if (!message) {
      return;
    }

    pendingFocusIndexRef.current = boundedIndex;
    onSelectMessage(message.id);
    rowVirtualizer.scrollToIndex(boundedIndex, { align: "auto" });
    if (!messageButtonRefs.current[boundedIndex]) {
      scrollElementToMessageOffset(
        listRef.current,
        boundedIndex * MESSAGE_ROW_HEIGHT_PX,
      );
    }
    messageButtonRefs.current[boundedIndex]?.focus();
  }

  function handleVirtualMessageKeyDown(
    event: KeyboardEvent<HTMLButtonElement>,
    currentIndex: number,
    messageId: string,
  ) {
    const nextIndex = nextIndexForKey(
      event.key,
      currentIndex,
      visibleMessages.length,
    );

    if (nextIndex !== null) {
      event.preventDefault();
      focusMessageAtIndex(nextIndex);
      return;
    }

    if (isActivationKey(event.key)) {
      event.preventDefault();
      onReadMessage(messageId);
    }
  }

  useEffect(() => {
    const pendingFocusIndex = pendingFocusIndexRef.current;
    if (pendingFocusIndex === null) {
      return;
    }

    const messageButton = messageButtonRefs.current[pendingFocusIndex];
    if (!messageButton) {
      return;
    }

    messageButton.focus();
    pendingFocusIndexRef.current = null;
  }, [state.selectedMessageId, virtualRows]);

  useEffect(() => {
    if (
      isBackendSearchActive ||
      !onLoadMoreMessages ||
      paginationStatus.status === "loading" ||
      !paginationStatus.hasMore ||
      visibleMessages.length === 0
    ) {
      return;
    }

    const lastVirtualRow = virtualRows[virtualRows.length - 1];
    if (
      lastVirtualRow &&
      visibleMessages.length - lastVirtualRow.index <= MESSAGE_PAGE_LOAD_AHEAD_ROWS
    ) {
      onLoadMoreMessages();
    }
  }, [
    isBackendSearchActive,
    onLoadMoreMessages,
    paginationStatus.hasMore,
    paginationStatus.status,
    virtualRows,
    visibleMessages.length,
  ]);

  if (state.loadState.messages === "loading") {
    return (
      <div className="messages">
        <section
          className="empty-panel"
          aria-label={t("mail.states.messagesLoading.label")}
        >
          <h3>{t("mail.states.messagesLoading.heading")}</h3>
          <p>{t("mail.states.messagesLoading.body")}</p>
        </section>
      </div>
    );
  }

  if (state.loadState.messages === "error") {
    return (
      <div className="messages">
        <section
          className="empty-panel error-panel"
          aria-label={t("mail.states.messagesError.label")}
        >
          <h3>{t("mail.states.messagesError.heading")}</h3>
          <p>{t("mail.states.messagesError.body")}</p>
          <button type="button" onClick={onRetry} disabled={!onRetry}>
            {t("common.actions.retry")}
          </button>
        </section>
      </div>
    );
  }

  if (state.data.accounts.length === 0) {
    return (
      <div className="messages">
        <section
          className="empty-panel"
          aria-label={t("mail.states.emptyInbox.label")}
        >
          <h3>{t("mail.states.emptyInbox.heading")}</h3>
          <p>{t("mail.states.emptyInbox.body")}</p>
        </section>
      </div>
    );
  }

  if (
    isBackendSearchActive &&
    (searchStatus === "debouncing" || searchStatus === "loading")
  ) {
    return (
      <div className="messages">
        <section
          className="empty-panel"
          aria-label={t("mail.search.loading.label")}
        >
          <h3>{t("mail.search.loading.heading")}</h3>
          <p>{t("mail.search.loading.body")}</p>
        </section>
      </div>
    );
  }

  if (isBackendSearchActive && searchStatus === "unavailable") {
    return (
      <div className="messages">
        <section
          className="empty-panel error-panel"
          aria-label={t("mail.search.indexUnavailable.label")}
        >
          <h3>{t("mail.search.indexUnavailable.heading")}</h3>
          <p>{t("mail.search.indexUnavailable.body")}</p>
        </section>
      </div>
    );
  }

  if (isBackendSearchActive && searchStatus === "error") {
    return (
      <div className="messages">
        <section
          className="empty-panel error-panel"
          aria-label={t("mail.search.error.label")}
        >
          <h3>{t("mail.search.error.heading")}</h3>
          <p>{t("mail.search.error.body")}</p>
          <button type="button" onClick={onRetrySearch}>
            {t("common.actions.retry")}
          </button>
        </section>
      </div>
    );
  }

  if (visibleMessages.length === 0 && state.searchQuery.trim()) {
    return (
      <div className="messages">
        <section
          className="empty-panel"
          aria-label={t("mail.search.noMatches.label")}
        >
          <h3>{t("mail.search.noMatches.heading")}</h3>
          <p>{t("mail.search.noMatches.body")}</p>
        </section>
      </div>
    );
  }

  if (visibleMessages.length === 0) {
    return (
      <div className="messages">
        <section
          className="empty-panel"
          aria-label={t("mail.states.emptyFolder.label")}
        >
          <h3>{t("mail.states.emptyFolder.heading")}</h3>
          <p>{t("mail.states.emptyFolder.body")}</p>
        </section>
      </div>
    );
  }

  return (
    <div
      className="messages"
      data-message-count={visibleMessages.length}
      data-testid="message-virtual-list"
      ref={listRef}
    >
      <div
        className="messages-virtual-spacer"
        style={{ height: `${rowVirtualizer.getTotalSize()}px` }}
      >
        {virtualRows.map((virtualRow) => {
          const message = visibleMessages[virtualRow.index];
          if (!message) {
            return null;
          }
          const locationLabel = state.searchQuery.trim()
            ? messageLocationLabel(message, accountById, folderById, t)
            : null;

          return (
            <div
              className="message-virtual-row"
              key={virtualRow.key}
              style={{ transform: `translateY(${virtualRow.start}px)` }}
            >
              <MessageRow
                index={virtualRow.index}
                isSelected={message.id === state.selectedMessageId}
                locationLabel={locationLabel}
                message={message}
                searchHighlightTerms={searchHighlightTerms}
                refCallback={(element) => {
                  messageButtonRefs.current[virtualRow.index] = element;
                }}
                onClick={() => onSelectMessage(message.id)}
                onKeyDown={(event) =>
                  handleVirtualMessageKeyDown(
                    event,
                    virtualRow.index,
                    message.id,
                  )
                }
                rowCount={visibleMessages.length}
              />
            </div>
          );
        })}
      </div>
      {!isBackendSearchActive ? (
        <MessagePaginationStatus
          loadedCount={visibleMessages.length}
          onRetry={onRetryPage}
          status={paginationStatus}
        />
      ) : null}
    </div>
  );
}

function MessagePaginationStatus({
  loadedCount,
  onRetry,
  status,
}: {
  loadedCount: number;
  onRetry: () => void;
  status: MailMessagePageUiState;
}) {
  const { t } = useTranslation();
  const localeFormat = useLocaleFormatters();
  if (!status.scopeKey) {
    return null;
  }

  if (status.status === "loading") {
    return (
      <p className="message-pagination-status" role="status">
        {t("mail.pagination.loading")}
      </p>
    );
  }

  if (status.status === "error") {
    return (
      <div
        className="message-pagination-status message-pagination-status--error"
        role="alert"
      >
        <span>{t("mail.pagination.error")}</span>
        <button type="button" onClick={onRetry}>
          {t("common.actions.retry")}
        </button>
      </div>
    );
  }

  if (
    !status.hasMore &&
    status.totalCount !== null &&
    loadedCount >= status.totalCount
  ) {
    return (
      <p className="message-pagination-status" role="status">
        {t("mail.pagination.complete", {
          count: status.totalCount,
          formattedCount: localeFormat.count(loadedCount),
        })}
      </p>
    );
  }

  return null;
}

function MessageRow({
  index,
  isSelected,
  locationLabel,
  message,
  onClick,
  onKeyDown,
  refCallback,
  rowCount,
  searchHighlightTerms,
}: {
  index: number;
  isSelected: boolean;
  locationLabel: string | null;
  message: MailMessage;
  onClick: () => void;
  onKeyDown: (event: KeyboardEvent<HTMLButtonElement>) => void;
  refCallback: (element: HTMLButtonElement | null) => void;
  rowCount: number;
  searchHighlightTerms: string[];
}) {
  const { t } = useTranslation();
  const localeFormat = useLocaleFormatters();
  const subject = message.subject || t("mail.messages.noSubject");
  const sender = messageSenderLabel(message, t);
  const receivedAt = localeFormat.relativeDateTime(message.receivedAt);
  const absoluteReceivedAt = localeFormat.dateTime(message.receivedAt);

  return (
    <button
      className={message.unread ? "message is-unread" : "message"}
      type="button"
      aria-current={isSelected ? "true" : undefined}
      aria-label={t("mail.messages.aria", {
        flagged: message.flagged ? t("mail.messages.flaggedAria") : "",
        location: locationLabel ? `, ${locationLabel}` : "",
        readState: message.unread
          ? t("mail.messages.unread")
          : t("mail.messages.read"),
        receivedAt,
        sender,
        subject,
      })}
      aria-posinset={index + 1}
      aria-setsize={rowCount}
      data-message-index={index}
      ref={refCallback}
      onKeyDown={onKeyDown}
      onClick={onClick}
    >
      <div className="message-row">
        <strong>
          <HighlightedText terms={searchHighlightTerms} text={sender} />
        </strong>
        <span className="message-row-meta">
          {message.flagged ? (
            <span className="message-flag">{t("mail.messages.flagged")}</span>
          ) : null}
          {locationLabel ? (
            <span className="message-location">{locationLabel}</span>
          ) : null}
          <time
            dateTime={message.receivedAt}
            title={absoluteReceivedAt || undefined}
          >
            {receivedAt}
          </time>
        </span>
      </div>
      <h3>
        <HighlightedText terms={searchHighlightTerms} text={subject} />
      </h3>
      <p>
        <HighlightedText terms={searchHighlightTerms} text={message.preview} />
      </p>
    </button>
  );
}

function messageLocationLabel(
  message: MailMessage,
  accountById: Map<string, { label: string }>,
  folderById: Map<string, { name: string }>,
  t: TFunction,
) {
  const account = accountById.get(message.accountId);
  const folder = folderById.get(message.folderId);

  return t("mail.messages.location", {
    account: account?.label ?? t("app.sections.accounts"),
    folder: folder?.name ?? t("app.sections.folders"),
  });
}

function observeMessageListRect(
  instance: Virtualizer<HTMLDivElement, HTMLButtonElement>,
  callback: (rect: Rect) => void,
) {
  const element = instance.scrollElement;
  if (!element) {
    return;
  }

  const readRect = () => {
    const rect = element.getBoundingClientRect();
    callback({
      width:
        Math.round(rect.width) ||
        element.clientWidth ||
        MESSAGE_LIST_FALLBACK_RECT.width,
      height:
        Math.round(rect.height) ||
        element.clientHeight ||
        MESSAGE_LIST_FALLBACK_RECT.height,
    });
  };

  readRect();

  const ownerWindow = element.ownerDocument.defaultView;
  if (!ownerWindow?.ResizeObserver) {
    return undefined;
  }

  const observer = new ownerWindow.ResizeObserver(readRect);
  observer.observe(element);

  return () => {
    observer.disconnect();
  };
}

function scrollMessageListToOffset(
  offset: number,
  {
    adjustments = 0,
    behavior,
  }: {
    adjustments?: number;
    behavior?: ScrollBehavior | "instant";
  },
  instance: Virtualizer<HTMLDivElement, HTMLButtonElement>,
) {
  const element = instance.scrollElement;
  if (!element) {
    return;
  }

  const top = offset + adjustments;
  scrollElementToMessageOffset(element, top, behavior);
}

function scrollElementToMessageOffset(
  element: HTMLDivElement | null,
  top: number,
  behavior?: ScrollBehavior | "instant",
) {
  if (!element) {
    return;
  }

  if (typeof element.scrollTo === "function") {
    element.scrollTo({ top, behavior });
  }

  if (element.scrollTop !== top) {
    element.scrollTop = top;
    element.dispatchEvent(
      new (element.ownerDocument.defaultView?.Event ?? Event)("scroll", {
        bubbles: true,
      }),
    );
  }
}
