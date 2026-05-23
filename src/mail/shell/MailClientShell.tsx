import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  useVirtualizer,
  type Rect,
  type Virtualizer,
} from "@tanstack/react-virtual";
import { useLocaleFormatters } from "../../i18n";
import { SafeEmailBody } from "../rendering/SafeEmailBody";
import {
  TipTapComposer,
  type ComposerAttachmentStageFunction,
  type ComposerDraftDiscardFunction,
  type ComposerDraftSaveFunction,
  type RecentRecipientSearchFunction,
  type ComposerSendFunction,
} from "../composer/TipTapComposer";
import { AccountSignatureSettings } from "../../settings/AccountSignatureSettings";
import {
  createMailShellState,
  emptyMailShellData,
  errorMailShellLoadState,
  foldersForScope,
  loadingMailShellLoadState,
  mergeMailShellData,
  messagesForScope,
  readyMailShellLoadState,
  selectAccountScope,
  selectFolderScope,
  selectUnifiedScope,
  selectedAccount,
  selectedFolder,
  selectedMessage,
  type MailShellData,
  type MailShellLoadState,
  type MailAttachment,
  type MailSearchRequest,
  type MailSearchResponse,
  type MailMessage,
  type MailShellState,
} from "./mailShellState";

export type MailSearchFunction = (
  request: MailSearchRequest,
  signal: AbortSignal,
) => Promise<MailSearchResponse>;

export type MailClientShellProps = {
  initialData?: MailShellData;
  initialLoadState?: MailShellLoadState;
  loadMailShellData?: () => Promise<MailShellData>;
  searchMail?: MailSearchFunction;
  searchDebounceMs?: number;
  loadRecentRecipients?: RecentRecipientSearchFunction;
  saveComposerDraft?: ComposerDraftSaveFunction;
  discardComposerDraft?: ComposerDraftDiscardFunction;
  stageComposerAttachment?: ComposerAttachmentStageFunction;
  sendComposerMail?: ComposerSendFunction;
};

type Drawer = "composer" | "settings" | null;
type MailSearchUiStatus =
  | "idle"
  | "debouncing"
  | "loading"
  | "ready"
  | "error"
  | "unavailable";
type MailSearchUiState = {
  query: string;
  results: MailMessage[];
  status: MailSearchUiStatus;
};

const MESSAGE_ROW_HEIGHT_PX = 136;
const MESSAGE_LIST_OVERSCAN = 8;
const MAIL_SEARCH_DEBOUNCE_MS = 250;
const MAIL_SEARCH_RESULT_LIMIT = 100;
const MESSAGE_LIST_FALLBACK_RECT: Rect = {
  width: 420,
  height: 640,
};
const emptySearchState: MailSearchUiState = {
  query: "",
  results: [],
  status: "idle",
};

export function MailClientShell({
  initialData = emptyMailShellData,
  initialLoadState = readyMailShellLoadState,
  loadMailShellData,
  searchMail,
  searchDebounceMs = MAIL_SEARCH_DEBOUNCE_MS,
  loadRecentRecipients,
  saveComposerDraft,
  discardComposerDraft,
  stageComposerAttachment,
  sendComposerMail,
}: MailClientShellProps) {
  const { t } = useTranslation();
  const localeFormat = useLocaleFormatters();
  const [state, setState] = useState<MailShellState>(() =>
    createMailShellState(
      initialData,
      loadMailShellData ? loadingMailShellLoadState() : initialLoadState,
    ),
  );
  const [searchState, setSearchState] =
    useState<MailSearchUiState>(emptySearchState);
  const [searchRetryNonce, setSearchRetryNonce] = useState(0);
  const [drawer, setDrawer] = useState<Drawer>(null);
  const accountButtonRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const folderButtonRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const readerRef = useRef<HTMLElement | null>(null);
  const composeButtonRef = useRef<HTMLButtonElement | null>(null);
  const settingsButtonRef = useRef<HTMLButtonElement | null>(null);
  const drawerRef = useRef<HTMLDivElement | null>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);
  const visibleFolders = useMemo(() => foldersForScope(state), [state]);
  const scopedMessages = useMemo(() => messagesForScope(state), [state]);
  const searchQuery = state.searchQuery.trim();
  const isBackendSearchActive = Boolean(searchMail && searchQuery);
  const searchAccountId = state.unifiedInbox ? null : state.selectedAccountId;
  const searchFolderId = state.selectedFolderId;
  const hasFreshSearchResults = searchState.query === searchQuery;
  const effectiveSearchStatus: MailSearchUiStatus = isBackendSearchActive
    ? hasFreshSearchResults
      ? searchState.status
      : "debouncing"
    : "idle";
  const effectiveSearchResults =
    isBackendSearchActive && hasFreshSearchResults ? searchState.results : [];
  const visibleMessages = isBackendSearchActive
    ? effectiveSearchResults
    : scopedMessages;
  const activeMessage = state.selectedMessageId
    ? visibleMessages.find((message) => message.id === state.selectedMessageId) ??
      selectedMessage(state)
    : null;
  const activeAccount = selectedAccount(state);
  const activeFolder = selectedFolder(state);
  const accountById = useMemo(
    () => new Map(state.data.accounts.map((account) => [account.id, account])),
    [state.data.accounts],
  );
  const folderById = useMemo(
    () => new Map(state.data.folders.map((folder) => [folder.id, folder])),
    [state.data.folders],
  );
  const hasAccounts = state.data.accounts.length > 0;
  const canCompose = hasAccounts && state.loadState.accounts === "ready";
  const listTitle = state.unifiedInbox
    ? t("mail.folders.unifiedInbox")
    : activeFolder?.name ?? activeAccount?.label ?? t("mail.folders.allMail");

  function patchState(update: Partial<MailShellState>) {
    setState((current) => ({
      ...current,
      ...update,
    }));
  }

  function openDrawer(nextDrawer: Drawer, restoreFocus: HTMLElement | null) {
    restoreFocusRef.current = restoreFocus;
    setDrawer(nextDrawer);
  }

  function closeDrawer() {
    const restoreFocus = restoreFocusRef.current;
    setDrawer(null);
    restoreFocusRef.current = null;
    restoreFocus?.focus();
  }

  function focusSearch() {
    if (!hasAccounts || state.loadState.messages !== "ready") {
      return;
    }

    searchInputRef.current?.focus();
  }

  function handleShellKeyDown(event: KeyboardEvent<HTMLElement>) {
    if (drawer || isEditableTarget(event.target)) {
      return;
    }

    if (
      ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") ||
      event.key === "/"
    ) {
      event.preventDefault();
      focusSearch();
    }
  }

  function moveAccountFocus(nextIndex: number) {
    const boundedIndex = clampIndex(nextIndex, state.data.accounts.length);
    const account = state.data.accounts[boundedIndex];
    if (!account) {
      return;
    }

    setState((current) => selectAccountScope(current, account.id));
    accountButtonRefs.current[boundedIndex]?.focus();
  }

  function handleAccountKeyDown(
    event: KeyboardEvent<HTMLButtonElement>,
    currentIndex: number,
  ) {
    const nextIndex = nextIndexForKey(
      event.key,
      currentIndex,
      state.data.accounts.length,
    );

    if (nextIndex === null) {
      return;
    }

    event.preventDefault();
    moveAccountFocus(nextIndex);
  }

  function moveFolderFocus(nextIndex: number) {
    const itemCount = visibleFolders.length + 1;
    const boundedIndex = clampIndex(nextIndex, itemCount);
    const folder = boundedIndex === 0 ? null : visibleFolders[boundedIndex - 1];

    setState((current) => selectFolderScope(current, folder?.id ?? null));
    folderButtonRefs.current[boundedIndex]?.focus();
  }

  function handleFolderKeyDown(
    event: KeyboardEvent<HTMLButtonElement>,
    currentIndex: number,
  ) {
    const nextIndex = nextIndexForKey(
      event.key,
      currentIndex,
      visibleFolders.length + 1,
    );

    if (nextIndex === null) {
      return;
    }

    event.preventDefault();
    moveFolderFocus(nextIndex);
  }

  function handleDrawerKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      closeDrawer();
      return;
    }

    if (event.key !== "Tab") {
      return;
    }

    const focusableElements = getFocusableElements(drawerRef.current);
    if (focusableElements.length === 0) {
      event.preventDefault();
      drawerRef.current?.focus();
      return;
    }

    const firstElement = focusableElements[0];
    const lastElement = focusableElements[focusableElements.length - 1];
    const activeElement = document.activeElement;

    if (event.shiftKey && activeElement === firstElement) {
      event.preventDefault();
      lastElement.focus();
      return;
    }

    if (!event.shiftKey && activeElement === lastElement) {
      event.preventDefault();
      firstElement.focus();
    }
  }

  async function reloadMailShell() {
    if (!loadMailShellData) {
      return;
    }

    setState((current) => ({
      ...current,
      loadState: loadingMailShellLoadState(),
    }));

    try {
      const data = await loadMailShellData();
      setState((current) =>
        mergeMailShellData(current, data, readyMailShellLoadState),
      );
    } catch {
      setState((current) => ({
        ...current,
        loadState: errorMailShellLoadState(),
      }));
    }
  }

  useEffect(() => {
    if (!loadMailShellData) {
      return;
    }

    let cancelled = false;

    loadMailShellData()
      .then((data) => {
        if (cancelled) {
          return;
        }

        setState((current) =>
          mergeMailShellData(current, data, readyMailShellLoadState),
        );
      })
      .catch(() => {
        if (cancelled) {
          return;
        }

        setState((current) => ({
          ...current,
          loadState: errorMailShellLoadState(),
        }));
      });

    return () => {
      cancelled = true;
    };
  }, [loadMailShellData]);

  useEffect(() => {
    if (
      !searchMail ||
      !searchQuery ||
      !hasAccounts ||
      state.loadState.messages !== "ready"
    ) {
      return;
    }

    const request = searchRequestForScope(
      searchQuery,
      searchAccountId,
      searchFolderId,
    );
    const controller = new AbortController();
    const debounceHandle = window.setTimeout(() => {
      setSearchState((current) => ({
        query: searchQuery,
        results: current.query === searchQuery ? current.results : [],
        status: "loading",
      }));

      searchMail(request, controller.signal)
        .then((response) => {
          if (controller.signal.aborted) {
            return;
          }

          setSearchState({
            query: response.query,
            results: response.results,
            status:
              response.indexStatus === "unavailable" ? "unavailable" : "ready",
          });
        })
        .catch((error: unknown) => {
          if (controller.signal.aborted || isAbortError(error)) {
            return;
          }

          setSearchState({
            query: searchQuery,
            results: [],
            status: "error",
          });
        });
    }, searchDebounceMs);

    return () => {
      controller.abort();
      window.clearTimeout(debounceHandle);
    };
  }, [
    hasAccounts,
    searchDebounceMs,
    searchMail,
    searchAccountId,
    searchFolderId,
    searchQuery,
    searchRetryNonce,
    state.loadState.messages,
  ]);

  useEffect(() => {
    if (!drawer) {
      return;
    }

    const dialog = drawerRef.current;
    const preferredFocus = dialog?.querySelector<HTMLElement>(
      "[data-autofocus='true']",
    );

    (preferredFocus ?? getFocusableElements(dialog)[0] ?? dialog)?.focus();
  }, [drawer]);

  return (
    <main className="app-shell" onKeyDown={handleShellKeyDown}>
      <aside className="sidebar" aria-label={t("app.sections.mailboxes")}>
        <div className="brand">
          <span className="brand-mark" aria-hidden="true">
            K
          </span>
          <div>
            <h1>Komorebi</h1>
            <p>{t("app.brandSubtitle")}</p>
          </div>
        </div>

        <section className="sidebar-section" aria-label={t("app.sections.accounts")}>
          <div className="section-heading">
            <h2>{t("app.sections.accounts")}</h2>
            <button
              className="icon-command"
              type="button"
              aria-label={t("mail.account.add")}
              disabled
            >
              +
            </button>
          </div>

          <label className="unified-toggle">
            <input
              type="checkbox"
              checked={state.unifiedInbox}
              disabled={!hasAccounts}
              onChange={(event) =>
                setState((current) =>
                  event.target.checked
                    ? selectUnifiedScope(current)
                    : selectAccountScope(
                        current,
                        current.selectedAccountId ??
                          current.data.accounts[0]?.id ??
                          "",
                      ),
                )
              }
            />
            <span>{t("mail.unifiedToggle")}</span>
          </label>

          <StatusList
            status={state.loadState.accounts}
            loadingLabel={t("mail.states.accountsLoading")}
            errorLabel={t("mail.states.accountsError")}
            empty={state.data.accounts.length === 0}
            emptyLabel={t("mail.states.noAccounts")}
            onRetry={loadMailShellData ? reloadMailShell : undefined}
          >
            {state.data.accounts.map((account, index) => (
              <button
                className={
                  !state.unifiedInbox && account.id === state.selectedAccountId
                    ? "account-button is-active"
                    : "account-button"
                }
                type="button"
                key={account.id}
                aria-label={t("mail.account.aria", {
                  email: account.email,
                  label: account.label,
                  status: accountStatusLabel(account.status, t),
                  unread: localeFormat.count(account.unreadCount),
                })}
                ref={(element) => {
                  accountButtonRefs.current[index] = element;
                }}
                onKeyDown={(event) =>
                  handleAccountKeyDown(event, index)
                }
                onClick={() =>
                  setState((current) => selectAccountScope(current, account.id))
                }
              >
                <span>{account.label}</span>
                <small>{account.email}</small>
                <span className="account-meta">
                  <strong>{accountStatusLabel(account.status, t)}</strong>
                  <span>{localeFormat.unreadCount(account.unreadCount)}</span>
                </span>
              </button>
            ))}
          </StatusList>
        </section>

        <nav className="sidebar-section" aria-label={t("app.sections.folders")}>
          <h2>{t("app.sections.folders")}</h2>
          <StatusList
            status={state.loadState.folders}
            loadingLabel={t("mail.states.foldersLoading")}
            errorLabel={t("mail.states.foldersError")}
            empty={visibleFolders.length === 0}
            emptyLabel={t("mail.states.foldersEmpty")}
            onRetry={loadMailShellData ? reloadMailShell : undefined}
          >
            <button
              className={!state.selectedFolderId ? "folder is-active" : "folder"}
              type="button"
              aria-label={t("mail.folders.rootAria", {
                name: state.unifiedInbox
                  ? t("mail.folders.unifiedInbox")
                  : t("mail.folders.allMail"),
                unread: localeFormat.count(unreadCountForScope(state)),
              })}
              ref={(element) => {
                folderButtonRefs.current[0] = element;
              }}
              onKeyDown={(event) => handleFolderKeyDown(event, 0)}
              onClick={() => setState((current) => selectFolderScope(current, null))}
            >
              <span>
                {state.unifiedInbox
                  ? t("mail.folders.unifiedInbox")
                  : t("mail.folders.allMail")}
              </span>
              <span>{localeFormat.count(unreadCountForScope(state))}</span>
            </button>
            {visibleFolders.map((folder, index) => (
              <button
                className={
                  folder.id === state.selectedFolderId
                    ? "folder is-active"
                    : "folder"
                }
                type="button"
                key={folder.id}
                aria-label={t("mail.folders.aria", {
                  name: folder.name,
                  total: localeFormat.count(folder.totalCount),
                  unread: localeFormat.count(folder.unreadCount),
                })}
                ref={(element) => {
                  folderButtonRefs.current[index + 1] = element;
                }}
                onKeyDown={(event) => handleFolderKeyDown(event, index + 1)}
                onClick={() =>
                  setState((current) => selectFolderScope(current, folder.id))
                }
              >
                <span>{folder.name}</span>
                <span>{localeFormat.count(folder.unreadCount)}</span>
              </button>
            ))}
          </StatusList>
        </nav>

        <section className="sidebar-section sidebar-section--footer" aria-label={t("app.sections.app")}>
          <button
            className="settings-button"
            type="button"
            ref={settingsButtonRef}
            onClick={() => openDrawer("settings", settingsButtonRef.current)}
          >
            {t("settings.section")}
          </button>
        </section>
      </aside>

      <section className="message-list" aria-label={t("app.sections.messages")}>
        <header className="list-toolbar">
          <div>
            <p className="eyebrow">
              {state.unifiedInbox ? t("app.scope.allAccounts") : activeAccount?.label ?? t("app.scope.local")}
            </p>
            <h2>{listTitle}</h2>
          </div>
          <button
            ref={composeButtonRef}
            type="button"
            disabled={!canCompose}
            onClick={() => openDrawer("composer", composeButtonRef.current)}
          >
            {t("composer.actions.compose")}
          </button>
        </header>

        <label className="search">
          <span>{t("mail.search.label")}</span>
          <input
            ref={searchInputRef}
            type="search"
            placeholder={t("mail.search.placeholder")}
            disabled={!hasAccounts || state.loadState.messages !== "ready"}
            value={state.searchQuery}
            onChange={(event) =>
              patchState({
                searchQuery: event.target.value,
                selectedMessageId: null,
              })
            }
          />
          <SearchStatusLine
            isBackendSearchActive={isBackendSearchActive}
            resultCount={visibleMessages.length}
            status={effectiveSearchStatus}
          />
        </label>

        <MessageListState
          accountById={accountById}
          folderById={folderById}
          isBackendSearchActive={isBackendSearchActive}
          state={state}
          searchStatus={effectiveSearchStatus}
          visibleMessages={visibleMessages}
          onRetry={loadMailShellData ? reloadMailShell : undefined}
          onRetrySearch={() => setSearchRetryNonce((current) => current + 1)}
          onSelectMessage={(messageId) =>
            patchState({
              selectedMessageId: messageId,
            })
          }
          onReadMessage={(messageId) => {
            patchState({
              selectedMessageId: messageId,
            });
            readerRef.current?.focus();
          }}
        />
      </section>

      <section
        className="reader"
        aria-label={t("app.sections.selectedMessage")}
        ref={readerRef}
        tabIndex={-1}
      >
        <ReaderPane
          state={state}
          message={activeMessage}
          hasVisibleMessages={visibleMessages.length > 0}
          onRetry={loadMailShellData ? reloadMailShell : undefined}
        />
      </section>

      {drawer === "settings" ? (
        <div
          className="settings-drawer"
          role="dialog"
          aria-modal="true"
          aria-label={t("settings.aria.panel")}
          ref={drawerRef}
          tabIndex={-1}
          onKeyDown={handleDrawerKeyDown}
        >
          <AccountSignatureSettings
            accounts={state.data.accounts.map((account) => ({
              id: account.id,
              label: account.label,
              email: account.email,
              signatureHtml: account.signatureHtml,
            }))}
            initialAccountId={state.selectedAccountId}
            onSaveSignature={(accountId, signatureHtml) => {
              setState((current) => ({
                ...current,
                data: {
                  ...current.data,
                  accounts: current.data.accounts.map((account) =>
                    account.id === accountId
                      ? { ...account, signatureHtml }
                      : account,
                  ),
                },
              }));
            }}
            onClose={closeDrawer}
          />
        </div>
      ) : null}

      {drawer === "composer" ? (
        <div
          className="composer-drawer"
          role="dialog"
          aria-modal="true"
          aria-label={t("composer.aria.composer")}
          ref={drawerRef}
          tabIndex={-1}
          onKeyDown={handleDrawerKeyDown}
        >
          <ComposerDrawer
            accounts={state.data.accounts}
            discardDraft={discardComposerDraft}
            initialAccountId={activeAccount?.id ?? state.data.accounts[0]?.id ?? null}
            loadRecentRecipients={loadRecentRecipients}
            onClose={closeDrawer}
            saveDraft={saveComposerDraft}
            sendMail={sendComposerMail}
            stageAttachment={stageComposerAttachment}
          />
        </div>
      ) : null}
    </main>
  );
}

function searchRequestForScope(
  query: string,
  accountId: string | null,
  folderId: string | null,
): MailSearchRequest {
  return {
    accountId,
    folderId,
    limit: MAIL_SEARCH_RESULT_LIMIT,
    query,
  };
}

function isAbortError(error: unknown) {
  return error instanceof DOMException && error.name === "AbortError";
}

function SearchStatusLine({
  isBackendSearchActive,
  resultCount,
  status,
}: {
  isBackendSearchActive: boolean;
  resultCount: number;
  status: MailSearchUiStatus;
}) {
  const { t } = useTranslation();
  const localeFormat = useLocaleFormatters();

  if (!isBackendSearchActive) {
    return null;
  }

  if (status === "debouncing" || status === "loading") {
    return (
      <span className="search-status" role="status">
        {t("mail.search.status.loading")}
      </span>
    );
  }

  if (status === "unavailable") {
    return (
      <span className="search-status search-status--warning" role="status">
        {t("mail.search.status.unavailable")}
      </span>
    );
  }

  if (status === "error") {
    return (
      <span className="search-status search-status--warning" role="status">
        {t("mail.search.error.heading")}
      </span>
    );
  }

  if (status === "ready") {
    return (
      <span className="search-status" role="status">
        {localeFormat.searchResults(resultCount)}
      </span>
    );
  }

  return null;
}

function StatusList({
  status,
  loadingLabel,
  errorLabel,
  empty,
  emptyLabel,
  children,
  onRetry,
}: {
  status: MailShellLoadState[keyof MailShellLoadState];
  loadingLabel: string;
  errorLabel: string;
  empty: boolean;
  emptyLabel: string;
  children: ReactNode;
  onRetry?: () => void;
}) {
  const { t } = useTranslation();

  if (status === "loading") {
    return <p className="empty-note status-note">{loadingLabel}</p>;
  }

  if (status === "error") {
    return (
      <div className="status-note status-note--error" role="status">
        <p>{errorLabel}</p>
        <button type="button" onClick={onRetry} disabled={!onRetry}>
          {t("common.actions.retry")}
        </button>
      </div>
    );
  }

  if (empty) {
    return <p className="empty-note">{emptyLabel}</p>;
  }

  return children;
}

function MessageListState({
  accountById,
  folderById,
  isBackendSearchActive,
  state,
  searchStatus,
  visibleMessages,
  onRetry,
  onRetrySearch,
  onSelectMessage,
  onReadMessage,
}: {
  accountById: Map<string, { label: string }>;
  folderById: Map<string, { name: string }>;
  isBackendSearchActive: boolean;
  state: MailShellState;
  searchStatus: MailSearchUiStatus;
  visibleMessages: ReturnType<typeof messagesForScope>;
  onRetry?: () => void;
  onRetrySearch: () => void;
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

  if (state.loadState.messages === "loading") {
    return (
      <div className="messages">
        <section className="empty-panel" aria-label={t("mail.states.messagesLoading.label")}>
          <h3>{t("mail.states.messagesLoading.heading")}</h3>
          <p>{t("mail.states.messagesLoading.body")}</p>
        </section>
      </div>
    );
  }

  if (state.loadState.messages === "error") {
    return (
      <div className="messages">
        <section className="empty-panel error-panel" aria-label={t("mail.states.messagesError.label")}>
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
        <section className="empty-panel" aria-label={t("mail.states.emptyInbox.label")}>
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
        <section className="empty-panel" aria-label={t("mail.search.loading.label")}>
          <h3>{t("mail.search.loading.heading")}</h3>
          <p>{t("mail.search.loading.body")}</p>
        </section>
      </div>
    );
  }

  if (isBackendSearchActive && searchStatus === "unavailable") {
    return (
      <div className="messages">
        <section className="empty-panel error-panel" aria-label={t("mail.search.indexUnavailable.label")}>
          <h3>{t("mail.search.indexUnavailable.heading")}</h3>
          <p>{t("mail.search.indexUnavailable.body")}</p>
        </section>
      </div>
    );
  }

  if (isBackendSearchActive && searchStatus === "error") {
    return (
      <div className="messages">
        <section className="empty-panel error-panel" aria-label={t("mail.search.error.label")}>
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
        <section className="empty-panel" aria-label={t("mail.search.noMatches.label")}>
          <h3>{t("mail.search.noMatches.heading")}</h3>
          <p>{t("mail.search.noMatches.body")}</p>
        </section>
      </div>
    );
  }

  if (visibleMessages.length === 0) {
    return (
      <div className="messages">
        <section className="empty-panel" aria-label={t("mail.states.emptyFolder.label")}>
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
    </div>
  );
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
}: {
  index: number;
  isSelected: boolean;
  locationLabel: string | null;
  message: MailMessage;
  onClick: () => void;
  onKeyDown: (event: KeyboardEvent<HTMLButtonElement>) => void;
  refCallback: (element: HTMLButtonElement | null) => void;
  rowCount: number;
}) {
  const { t } = useTranslation();
  const localeFormat = useLocaleFormatters();
  const subject = message.subject || t("mail.messages.noSubject");
  const sender = messageSenderLabel(message, t);
  const receivedAt = localeFormat.dateTime(message.receivedAt);

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
        <strong>{sender}</strong>
        <span className="message-row-meta">
          {message.flagged ? (
            <span className="message-flag">{t("mail.messages.flagged")}</span>
          ) : null}
          {locationLabel ? (
            <span className="message-location">{locationLabel}</span>
          ) : null}
          <time dateTime={message.receivedAt}>{receivedAt}</time>
        </span>
      </div>
      <h3>{subject}</h3>
      <p>{message.preview}</p>
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

function messageSenderLabel(
  message: Pick<MailMessage, "sender" | "senderEmail">,
  t: TFunction,
) {
  return (
    message.sender.trim() ||
    message.senderEmail.trim() ||
    t("mail.messages.unknownSender")
  );
}

function ReaderPane({
  state,
  message,
  hasVisibleMessages,
  onRetry,
}: {
  state: MailShellState;
  message: ReturnType<typeof selectedMessage>;
  hasVisibleMessages: boolean;
  onRetry?: () => void;
}) {
  const { t } = useTranslation();
  const localeFormat = useLocaleFormatters();
  const sender = message ? messageSenderLabel(message, t) : "";

  if (state.loadState.messages === "loading") {
    return (
      <>
        <section className="reader-empty" aria-label={t("mail.reader.loading.label")}>
          <p className="eyebrow">{t("mail.reader.eyebrow")}</p>
          <h2>{t("mail.reader.loading.heading")}</h2>
          <p>{t("mail.reader.loading.body")}</p>
        </section>
        <ReaderStatus text={t("mail.reader.loading.status")} />
      </>
    );
  }

  if (state.loadState.messages === "error") {
    return (
      <>
        <section className="reader-empty" aria-label={t("mail.reader.error.label")}>
          <p className="eyebrow">{t("mail.reader.eyebrow")}</p>
          <h2>{t("mail.reader.error.heading")}</h2>
          <p>{t("mail.reader.error.body")}</p>
          <button type="button" onClick={onRetry} disabled={!onRetry}>
            {t("common.actions.retry")}
          </button>
        </section>
        <ReaderStatus text={t("common.status.errorState")} />
      </>
    );
  }

  if (!message) {
    return (
      <>
        <section className="reader-empty" aria-label={t("mail.reader.empty.label")}>
          <p className="eyebrow">{t("mail.reader.eyebrow")}</p>
          <h2>{t("mail.reader.empty.heading")}</h2>
          <p>
            {hasVisibleMessages
              ? t("mail.reader.empty.bodyWithMessages")
              : t("mail.reader.empty.body")}
          </p>
        </section>
        <ReaderStatus
          text={
            state.data.accounts.length === 0
              ? t("mail.reader.empty.accountSetupRequired")
              : t("mail.reader.remoteBlocked")
          }
        />
      </>
    );
  }

  return (
    <>
      <header className="reader-header">
        <p className="eyebrow">{t("mail.reader.selectedEyebrow")}</p>
        <h2>{message.subject || t("mail.messages.noSubject")}</h2>
        <div className="sender-line">
          <span>{sender}</span>
          <time dateTime={message.receivedAt}>
            {localeFormat.dateTime(message.receivedAt)}
          </time>
        </div>
      </header>

      <div className="reader-actions" aria-label={t("mail.reader.actions.label")}>
        <button
          type="button"
          disabled
          title={t("mail.reader.actions.replyUnavailableTitle")}
          aria-label={t("mail.reader.actions.replyUnavailable")}
        >
          {t("mail.reader.actions.reply")}
        </button>
        <button
          type="button"
          disabled
          title={t("mail.reader.actions.forwardUnavailableTitle")}
          aria-label={t("mail.reader.actions.forwardUnavailable")}
        >
          {t("mail.reader.actions.forward")}
        </button>
        <button
          type="button"
          disabled
          title={t("mail.reader.actions.archiveUnavailableTitle")}
          aria-label={t("mail.reader.actions.archiveUnavailable")}
        >
          {t("mail.reader.actions.archive")}
        </button>
        <button
          type="button"
          disabled
          title={t("mail.reader.actions.readStateUnavailableTitle")}
          aria-label={
            message.unread
              ? t("mail.reader.actions.markReadUnavailable")
              : t("mail.reader.actions.markUnreadUnavailable")
          }
        >
          {message.unread
            ? t("mail.reader.actions.markRead")
            : t("mail.reader.actions.markUnread")}
        </button>
        <button
          type="button"
          disabled
          title={t("mail.reader.actions.loadRemoteUnavailableTitle")}
          aria-label={t("mail.reader.actions.loadRemoteUnavailable")}
        >
          {t("mail.reader.actions.loadRemote")}
        </button>
      </div>

      <article className="reader-body">
        <SafeEmailBody bodyHtml={message.bodyHtml} bodyText={message.bodyText} />
        <AttachmentList attachments={message.attachments} />
      </article>

      <ReaderStatus text={t("mail.reader.remoteBlocked")} />
    </>
  );
}

function AttachmentList({ attachments }: { attachments: MailAttachment[] }) {
  const { t } = useTranslation();
  const localeFormat = useLocaleFormatters();

  if (attachments.length === 0) {
    return null;
  }

  return (
    <section className="attachment-list" aria-label={t("mail.attachments.heading")}>
      <h3>{t("mail.attachments.heading")}</h3>
      <ul>
        {attachments.map((attachment) => {
          const displayFilename =
            attachment.displayFilename.trim() ||
            t("mail.attachments.attachmentFallback");
          const contentType =
            attachment.contentType.trim() || "application/octet-stream";

          return (
            <li className="attachment-item" key={attachment.id}>
              <div className="attachment-meta">
                <strong>{displayFilename}</strong>
                <span>
                  {contentType} - {localeFormat.byteSize(attachment.sizeBytes)}
                </span>
                <small>{t(`mail.attachments.disposition.${attachment.disposition}`)}</small>
              </div>
              <div className="attachment-actions" aria-label={t("mail.attachments.actionsLabel")}>
                <button
                  type="button"
                  disabled
                  title={t("mail.attachments.previewUnavailableTitle")}
                  aria-label={t("mail.attachments.previewUnavailable", {
                    filename: displayFilename,
                  })}
                >
                  {t("mail.attachments.preview")}
                </button>
                <button
                  type="button"
                  disabled
                  title={t("mail.attachments.saveUnavailableTitle")}
                  aria-label={t("mail.attachments.saveUnavailable", {
                    filename: displayFilename,
                  })}
                >
                  {t("mail.attachments.save")}
                </button>
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function ReaderStatus({ text }: { text: string }) {
  const { t } = useTranslation();

  return (
    <footer className="reader-status">
      <span>{text}</span>
      <span>{t("common.status.localCache")}</span>
    </footer>
  );
}

const ComposerDrawer = TipTapComposer;

function accountStatusLabel(
  status: MailShellData["accounts"][number]["status"],
  t: TFunction,
) {
  return t(`mail.account.status.${status}`);
}

function unreadCountForScope(state: MailShellState) {
  const visibleFolders = foldersForScope(state);

  if (state.selectedFolderId) {
    return (
      visibleFolders.find((folder) => folder.id === state.selectedFolderId)
        ?.unreadCount ?? 0
    );
  }

  if (state.unifiedInbox) {
    return visibleFolders
      .filter((folder) => folder.kind === "inbox")
      .reduce((total, folder) => total + folder.unreadCount, 0);
  }

  return visibleFolders.reduce((total, folder) => total + folder.unreadCount, 0);
}

function nextIndexForKey(
  key: string,
  currentIndex: number,
  itemCount: number,
): number | null {
  switch (key) {
    case "ArrowDown":
    case "ArrowRight":
      return clampIndex(currentIndex + 1, itemCount);
    case "ArrowUp":
    case "ArrowLeft":
      return clampIndex(currentIndex - 1, itemCount);
    case "Home":
      return clampIndex(0, itemCount);
    case "End":
      return clampIndex(itemCount - 1, itemCount);
    default:
      return null;
  }
}

function clampIndex(index: number, itemCount: number) {
  if (itemCount <= 0) {
    return 0;
  }

  return Math.min(Math.max(index, 0), itemCount - 1);
}

function isActivationKey(key: string) {
  return key === "Enter" || key === " ";
}

function isEditableTarget(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) {
    return false;
  }

  const tagName = target.tagName.toLowerCase();

  return (
    target.isContentEditable ||
    tagName === "input" ||
    tagName === "select" ||
    tagName === "textarea"
  );
}

function getFocusableElements(container: HTMLElement | null) {
  if (!container) {
    return [];
  }

  const selectors = [
    "button:not([disabled])",
    "a[href]",
    "input:not([disabled])",
    "select:not([disabled])",
    "textarea:not([disabled])",
    "[tabindex]:not([tabindex='-1'])",
  ];

  return Array.from(
    container.querySelectorAll<HTMLElement>(selectors.join(",")),
  ).filter(
    (element) =>
      element.tabIndex >= 0 &&
      !element.hidden &&
      !element.closest("[hidden], [aria-hidden='true']"),
  );
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
