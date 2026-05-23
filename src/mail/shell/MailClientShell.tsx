import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import {
  useVirtualizer,
  type Rect,
  type Virtualizer,
} from "@tanstack/react-virtual";
import { SafeEmailBody } from "../rendering/SafeEmailBody";
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
}: MailClientShellProps) {
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
    ? "Unified Inbox"
    : activeFolder?.name ?? activeAccount?.label ?? "Inbox";

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
      <aside className="sidebar" aria-label="Mailboxes">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true">
            K
          </span>
          <div>
            <h1>Komorebi</h1>
            <p>Personal mail</p>
          </div>
        </div>

        <section className="sidebar-section" aria-label="Accounts">
          <div className="section-heading">
            <h2>Accounts</h2>
            <button
              className="icon-command"
              type="button"
              aria-label="Add account"
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
            <span>Unified inbox</span>
          </label>

          <StatusList
            status={state.loadState.accounts}
            loadingLabel="Loading accounts"
            errorLabel="Account list unavailable"
            empty={state.data.accounts.length === 0}
            emptyLabel="No accounts configured."
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
                aria-label={`${account.label} account, ${account.email}, ${account.unreadCount} unread, ${account.status}`}
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
                  <strong>{account.status}</strong>
                  <span>{account.unreadCount} unread</span>
                </span>
              </button>
            ))}
          </StatusList>
        </section>

        <nav className="sidebar-section" aria-label="Folders">
          <h2>Folders</h2>
          <StatusList
            status={state.loadState.folders}
            loadingLabel="Loading folders"
            errorLabel="Folder list unavailable"
            empty={visibleFolders.length === 0}
            emptyLabel="Folders appear after an account is added."
            onRetry={loadMailShellData ? reloadMailShell : undefined}
          >
            <button
              className={!state.selectedFolderId ? "folder is-active" : "folder"}
              type="button"
              aria-label={`${state.unifiedInbox ? "Unified Inbox" : "All mail"} folder, ${unreadCountForScope(state)} unread`}
              ref={(element) => {
                folderButtonRefs.current[0] = element;
              }}
              onKeyDown={(event) => handleFolderKeyDown(event, 0)}
              onClick={() => setState((current) => selectFolderScope(current, null))}
            >
              <span>{state.unifiedInbox ? "Unified Inbox" : "All mail"}</span>
              <span>{unreadCountForScope(state)}</span>
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
                aria-label={`${folder.name} folder, ${folder.unreadCount} unread, ${folder.totalCount} messages`}
                ref={(element) => {
                  folderButtonRefs.current[index + 1] = element;
                }}
                onKeyDown={(event) => handleFolderKeyDown(event, index + 1)}
                onClick={() =>
                  setState((current) => selectFolderScope(current, folder.id))
                }
              >
                <span>{folder.name}</span>
                <span>{folder.unreadCount}</span>
              </button>
            ))}
          </StatusList>
        </nav>

        <section className="sidebar-section sidebar-section--footer" aria-label="App">
          <button
            className="settings-button"
            type="button"
            ref={settingsButtonRef}
            onClick={() => openDrawer("settings", settingsButtonRef.current)}
          >
            Settings
          </button>
        </section>
      </aside>

      <section className="message-list" aria-label="Messages">
        <header className="list-toolbar">
          <div>
            <p className="eyebrow">
              {state.unifiedInbox ? "All accounts" : activeAccount?.label ?? "Local"}
            </p>
            <h2>{listTitle}</h2>
          </div>
          <button
            ref={composeButtonRef}
            type="button"
            disabled={!canCompose}
            onClick={() => openDrawer("composer", composeButtonRef.current)}
          >
            Compose
          </button>
        </header>

        <label className="search">
          <span>Search mail</span>
          <input
            ref={searchInputRef}
            type="search"
            placeholder="Sender, subject, or body"
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
        aria-label="Selected message"
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
          aria-label="Settings"
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
          aria-label="Composer"
          ref={drawerRef}
          tabIndex={-1}
          onKeyDown={handleDrawerKeyDown}
        >
          <ComposerDrawer
            accountLabel={
              activeAccount?.label ?? state.data.accounts[0]?.label ?? ""
            }
            onClose={closeDrawer}
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
  if (!isBackendSearchActive) {
    return null;
  }

  if (status === "debouncing" || status === "loading") {
    return (
      <span className="search-status" role="status">
        Searching local index
      </span>
    );
  }

  if (status === "unavailable") {
    return (
      <span className="search-status search-status--warning" role="status">
        Search index unavailable
      </span>
    );
  }

  if (status === "error") {
    return (
      <span className="search-status search-status--warning" role="status">
        Search unavailable
      </span>
    );
  }

  if (status === "ready") {
    return (
      <span className="search-status" role="status">
        {resultCount} {resultCount === 1 ? "result" : "results"}
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
  if (status === "loading") {
    return <p className="empty-note status-note">{loadingLabel}</p>;
  }

  if (status === "error") {
    return (
      <div className="status-note status-note--error" role="status">
        <p>{errorLabel}</p>
        <button type="button" onClick={onRetry} disabled={!onRetry}>
          Retry
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
        <section className="empty-panel" aria-label="Loading messages">
          <h3>Loading mail</h3>
          <p>Reading the local cache.</p>
        </section>
      </div>
    );
  }

  if (state.loadState.messages === "error") {
    return (
      <div className="messages">
        <section className="empty-panel error-panel" aria-label="Message error">
          <h3>Mail unavailable</h3>
          <p>The local message cache could not be opened.</p>
          <button type="button" onClick={onRetry} disabled={!onRetry}>
            Retry
          </button>
        </section>
      </div>
    );
  }

  if (state.data.accounts.length === 0) {
    return (
      <div className="messages">
        <section className="empty-panel" aria-label="Empty inbox">
          <h3>No mail loaded</h3>
          <p>Add an account to sync folders and messages into the local cache.</p>
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
        <section className="empty-panel" aria-label="Searching messages">
          <h3>Searching local index</h3>
          <p>Matching subject, sender, and body across active accounts.</p>
        </section>
      </div>
    );
  }

  if (isBackendSearchActive && searchStatus === "unavailable") {
    return (
      <div className="messages">
        <section className="empty-panel error-panel" aria-label="Search index unavailable">
          <h3>Search index unavailable</h3>
          <p>The local FTS index is missing or not ready.</p>
        </section>
      </div>
    );
  }

  if (isBackendSearchActive && searchStatus === "error") {
    return (
      <div className="messages">
        <section className="empty-panel error-panel" aria-label="Search error">
          <h3>Search unavailable</h3>
          <p>The local FTS query could not be completed.</p>
          <button type="button" onClick={onRetrySearch}>
            Retry
          </button>
        </section>
      </div>
    );
  }

  if (visibleMessages.length === 0 && state.searchQuery.trim()) {
    return (
      <div className="messages">
        <section className="empty-panel" aria-label="No search results">
          <h3>No matches</h3>
          <p>Search found no local messages in the selected scope.</p>
        </section>
      </div>
    );
  }

  if (visibleMessages.length === 0) {
    return (
      <div className="messages">
        <section className="empty-panel" aria-label="Empty folder">
          <h3>No messages</h3>
          <p>This folder has no local mail yet.</p>
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
            ? messageLocationLabel(message, accountById, folderById)
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
  return (
    <button
      className={message.unread ? "message is-unread" : "message"}
      type="button"
      aria-current={isSelected ? "true" : undefined}
      aria-label={`${message.unread ? "Unread" : "Read"}${
        message.flagged ? ", flagged" : ""
      } message from ${message.sender}, ${message.subject || "no subject"}, ${
        message.receivedAt
      }${locationLabel ? `, ${locationLabel}` : ""}`}
      aria-posinset={index + 1}
      aria-setsize={rowCount}
      data-message-index={index}
      ref={refCallback}
      onKeyDown={onKeyDown}
      onClick={onClick}
    >
      <div className="message-row">
        <strong>{message.sender}</strong>
        <span className="message-row-meta">
          {message.flagged ? (
            <span className="message-flag">Flagged</span>
          ) : null}
          {locationLabel ? (
            <span className="message-location">{locationLabel}</span>
          ) : null}
          <time>{message.receivedAt}</time>
        </span>
      </div>
      <h3>{message.subject || "(no subject)"}</h3>
      <p>{message.preview}</p>
    </button>
  );
}

function messageLocationLabel(
  message: MailMessage,
  accountById: Map<string, { label: string }>,
  folderById: Map<string, { name: string }>,
) {
  const account = accountById.get(message.accountId);
  const folder = folderById.get(message.folderId);

  return `${account?.label ?? "Account"} / ${folder?.name ?? "Folder"}`;
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
  if (state.loadState.messages === "loading") {
    return (
      <>
        <section className="reader-empty" aria-label="Reader loading">
          <p className="eyebrow">Reader</p>
          <h2>Loading message view</h2>
          <p>The local cache is being prepared.</p>
        </section>
        <ReaderStatus text="Loading local cache" />
      </>
    );
  }

  if (state.loadState.messages === "error") {
    return (
      <>
        <section className="reader-empty" aria-label="Reader error">
          <p className="eyebrow">Reader</p>
          <h2>Message view unavailable</h2>
          <p>Resolve the cache error before opening messages.</p>
          <button type="button" onClick={onRetry} disabled={!onRetry}>
            Retry
          </button>
        </section>
        <ReaderStatus text="Error state" />
      </>
    );
  }

  if (!message) {
    return (
      <>
        <section className="reader-empty" aria-label="No selected message">
          <p className="eyebrow">Reader</p>
          <h2>No message selected</h2>
          <p>
            {hasVisibleMessages
              ? "Select a message from the list to read it."
              : "Messages appear here after account setup and sync complete."}
          </p>
        </section>
        <ReaderStatus
          text={
            state.data.accounts.length === 0
              ? "Account setup required"
              : "Remote content blocked by default"
          }
        />
      </>
    );
  }

  return (
    <>
      <header className="reader-header">
        <p className="eyebrow">Selected message</p>
        <h2>{message.subject || "(no subject)"}</h2>
        <div className="sender-line">
          <span>{message.sender}</span>
          <time>{message.receivedAt}</time>
        </div>
      </header>

      <div className="reader-actions" aria-label="Message actions">
        <button
          type="button"
          disabled
          title="Composer reply routing is not implemented yet."
          aria-label="Reply unavailable"
        >
          Reply
        </button>
        <button
          type="button"
          disabled
          title="Composer forward routing is not implemented yet."
          aria-label="Forward unavailable"
        >
          Forward
        </button>
        <button
          type="button"
          disabled
          title="Archive is waiting for the typed mail-action command."
          aria-label="Archive unavailable"
        >
          Archive
        </button>
        <button
          type="button"
          disabled
          title="Read-state changes are waiting for the typed mail-action command."
          aria-label={`${message.unread ? "Mark read" : "Mark unread"} unavailable`}
        >
          {message.unread ? "Mark read" : "Mark unread"}
        </button>
        <button
          type="button"
          disabled
          title="Remote resources stay blocked until a per-message or per-sender preference command exists."
          aria-label="Load remote content unavailable; remote resources are blocked by default"
        >
          Load remote content
        </button>
      </div>

      <article className="reader-body">
        <SafeEmailBody bodyHtml={message.bodyHtml} bodyText={message.bodyText} />
        <AttachmentList attachments={message.attachments} />
      </article>

      <ReaderStatus text="Remote content blocked by default" />
    </>
  );
}

function AttachmentList({ attachments }: { attachments: MailAttachment[] }) {
  if (attachments.length === 0) {
    return null;
  }

  return (
    <section className="attachment-list" aria-label="Attachments">
      <h3>Attachments</h3>
      <ul>
        {attachments.map((attachment) => {
          const displayFilename =
            attachment.displayFilename.trim() || "Attachment";
          const contentType =
            attachment.contentType.trim() || "application/octet-stream";

          return (
            <li className="attachment-item" key={attachment.id}>
              <div className="attachment-meta">
                <strong>{displayFilename}</strong>
                <span>
                  {contentType} - {formatAttachmentSize(attachment.sizeBytes)}
                </span>
                <small>{attachment.disposition}</small>
              </div>
              <div className="attachment-actions" aria-label="Attachment actions">
                <button
                  type="button"
                  disabled
                  title="Attachment preview requires a typed backend command."
                  aria-label={`Preview ${displayFilename} unavailable`}
                >
                  Preview
                </button>
                <button
                  type="button"
                  disabled
                  title="Attachment saving requires a typed backend command."
                  aria-label={`Save ${displayFilename} unavailable`}
                >
                  Save
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
  return (
    <footer className="reader-status">
      <span>{text}</span>
      <span>Local cache</span>
    </footer>
  );
}

function ComposerDrawer({
  accountLabel,
  onClose,
}: {
  accountLabel: string;
  onClose: () => void;
}) {
  return (
    <section className="composer-panel" aria-label="Composer">
      <header className="settings-panel__header">
        <div>
          <p className="eyebrow">{accountLabel || "Composer"}</p>
          <h2>New message</h2>
        </div>
        <button type="button" onClick={onClose}>
          Close
        </button>
      </header>

      <form className="composer-form">
        <label>
          <span>To</span>
          <input
            type="text"
            placeholder="recipient@example.com"
            data-autofocus="true"
          />
        </label>
        <label>
          <span>Subject</span>
          <input type="text" />
        </label>
        <label>
          <span>Message</span>
          <textarea rows={10} />
        </label>

        <div className="settings-panel__actions">
          <button type="button">Save draft</button>
          <button type="button">Send</button>
        </div>
      </form>
    </section>
  );
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

function formatAttachmentSize(sizeBytes: number) {
  if (!Number.isFinite(sizeBytes) || sizeBytes <= 0) {
    return "0 B";
  }

  if (sizeBytes < 1024) {
    return `${Math.round(sizeBytes)} B`;
  }

  const units = ["KB", "MB", "GB"];
  let value = sizeBytes / 1024;
  let unitIndex = 0;

  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }

  const rounded = value >= 10 ? Math.round(value) : Math.round(value * 10) / 10;
  return `${rounded} ${units[unitIndex]}`;
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
