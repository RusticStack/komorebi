import {
  useEffect,
  lazy,
  Suspense,
  useCallback,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { Leaf, Plus } from "lucide-react";
import {
  detectInitialLanguage,
  i18n as komorebiI18n,
  useLocaleFormatters,
} from "../../i18n";
import { sanitizeEmailHtml, type SafeEmailLink } from "../rendering/emailHtml";
import {
  type ComposerAttachmentStageFunction,
  type ComposerDraftDiscardFunction,
  type ComposerDraftSaveFunction,
  type RecentRecipientSearchFunction,
  type ComposerSendFunction,
} from "../composer/TipTapComposer";
import { AccountSignatureSettings } from "../../settings/AccountSignatureSettings";
import { AccountStatusDrawer } from "./AccountStatusDrawer";
import { DraftsDrawer } from "./DraftsDrawer";
import { MailSidebar } from "./MailSidebar";
import { MessageListPane, type MailMessagePageUiState } from "./MessageListPane";
import { ReaderPane } from "./ReaderPane";
import { MailClientTitleBar } from "./MailClientTitleBar";
import {
  SearchQueryTools,
  SearchStatusLine,
  type MailSearchUiStatus,
} from "./MailSearchTools";
import {
  clampIndex,
  nextIndexForKey,
} from "./keyboardNavigation";
import { messageSenderLabel } from "./mailDisplay";
import { authStatusForManagement } from "./accountStatusDisplay";
import {
  accountLocalPreferences,
  createMailShellState,
  emptyMailShellData,
  errorMailShellLoadState,
  foldersForScope,
  loadingMailShellLoadState,
  mergeMailShellData,
  messagesForScope,
  readyMailShellLoadState,
  searchResultIsVisibleInScope,
  selectAccountScope,
  selectFolderScope,
  selectUnifiedScope,
  selectedAccount,
  selectedFolder,
  selectedMessage,
  type MailShellData,
  type MailShellLoadState,
  type MailAccount,
  type MailAttachment,
  type MailActionErrorResponse,
  type MailActionKind,
  type MailActionRequest,
  type MailActionResponse,
  type MailActionRetryDecision,
  type MailActionUndo,
  type MailSearchRequest,
  type MailSearchParsedQuery,
  type MailSearchResponse,
  type MailMessagePageCursor,
  type MailMessagePageRequest,
  type MailMessagePageResponse,
  type MailMessage,
  type MailShellState,
  type ComposerDraft,
  type ComposerAttachment,
  type ComposerForwardedAttachmentStageRequest,
  type AccountDiscoveryRequest,
  type AccountDiscoveryResponse,
  type AccountSetupRequest,
  type AccountSetupResponse,
  type AccountReauthRequest,
  type OAuthAccountReauthRequest,
  type OAuthAccountSetupRequest,
  type AccountRemovalRequest,
  type AccountRemovalResponse,
  type AccountConnectionTestResponse,
  type AccountAuthStatusRequest,
  type AccountAuthStatusView,
  type AccountSettingsUpdateRequest,
  type AccountSettingsUpdateResponse,
  type AttachmentCommandRequest,
  type AttachmentDownloadResponse,
  type AttachmentPreviewResponse,
  type AttachmentSaveResponse,
  type ExternalLinkOpenRequest,
  type ExternalLinkOpenResponse,
  type RemoteContentPreferenceRequest,
  type RemoteContentPreferenceResponse,
  type RemoteContentPreferenceScope,
  type UiPreferencesUpdateRequest,
  type UiPreferencesView,
  type DiagnosticsExportRequest,
  type DiagnosticsExportResponse,
  type DiagnosticsHealthView,
  type LoadStatus,
} from "./mailShellState";

export type MailSearchFunction = (
  request: MailSearchRequest,
  signal: AbortSignal,
) => Promise<MailSearchResponse>;
export type MailMessagePageFunction = (
  request: MailMessagePageRequest,
  signal: AbortSignal,
) => Promise<MailMessagePageResponse>;
export type RemoteContentPreferenceFunction = (
  request: RemoteContentPreferenceRequest,
) => Promise<RemoteContentPreferenceResponse>;
export type ExternalLinkOpenFunction = (
  request: ExternalLinkOpenRequest,
) => Promise<ExternalLinkOpenResponse>;
export type MailActionFunction = (
  request: MailActionRequest,
) => Promise<MailActionResponse>;
export type AccountCreateFunction = (
  request: AccountSetupRequest,
) => Promise<AccountSetupResponse>;
export type AccountDiscoveryFunction = (
  request: AccountDiscoveryRequest,
) => Promise<AccountDiscoveryResponse>;
export type AccountReauthFunction = (
  request: AccountReauthRequest,
) => Promise<AccountSetupResponse>;
export type OAuthAccountSetupFunction = (
  request: OAuthAccountSetupRequest,
) => Promise<AccountSetupResponse>;
export type OAuthAccountReauthFunction = (
  request: OAuthAccountReauthRequest,
) => Promise<AccountSetupResponse>;
export type AccountRemoveFunction = (
  request: AccountRemovalRequest,
) => Promise<AccountRemovalResponse>;
export type AccountConnectionTestFunction = (
  request: AccountSetupRequest,
) => Promise<AccountConnectionTestResponse>;
export type AccountAuthStatusFunction = (
  request: AccountAuthStatusRequest,
) => Promise<AccountAuthStatusView>;
export type AccountSettingsUpdateFunction = (
  request: AccountSettingsUpdateRequest,
) => Promise<AccountSettingsUpdateResponse>;
export type UiPreferencesLoadFunction = () => Promise<UiPreferencesView>;
export type UiPreferencesUpdateFunction = (
  request: UiPreferencesUpdateRequest,
) => Promise<UiPreferencesView>;
export type DiagnosticsExportFunction = (
  request: DiagnosticsExportRequest,
) => Promise<DiagnosticsExportResponse>;
export type DiagnosticsHealthLoadFunction = () => Promise<DiagnosticsHealthView>;
export type ComposerDraftListFunction = () => Promise<ComposerDraft[]>;
export type ComposerForwardedAttachmentStageFunction = (
  request: ComposerForwardedAttachmentStageRequest,
) => Promise<ComposerAttachment[]>;
export type AttachmentPreviewFunction = (
  request: AttachmentCommandRequest,
) => Promise<AttachmentPreviewResponse>;
export type AttachmentSaveFunction = (
  request: AttachmentCommandRequest,
) => Promise<AttachmentSaveResponse>;
export type AttachmentDownloadFunction = (
  request: AttachmentCommandRequest,
) => Promise<AttachmentDownloadResponse>;
export type SyncStatusUnsubscribeFunction = () => void | Promise<void>;
export type SyncStatusSubscribeFunction = (
  handler: (payload: unknown) => void,
) => SyncStatusUnsubscribeFunction | Promise<SyncStatusUnsubscribeFunction>;

export type MailClientShellProps = {
  initialData?: MailShellData;
  initialComposerDrafts?: ComposerDraft[];
  initialLoadState?: MailShellLoadState;
  loadMailShellData?: () => Promise<MailShellData>;
  loadMessagePage?: MailMessagePageFunction;
  loadComposerDrafts?: ComposerDraftListFunction;
  searchMail?: MailSearchFunction;
  allowRemoteContent?: RemoteContentPreferenceFunction;
  loadUiPreferences?: UiPreferencesLoadFunction;
  updateUiPreferences?: UiPreferencesUpdateFunction;
  openExternalLink?: ExternalLinkOpenFunction;
  performMailAction?: MailActionFunction;
  previewAttachment?: AttachmentPreviewFunction;
  saveAttachment?: AttachmentSaveFunction;
  downloadAttachment?: AttachmentDownloadFunction;
  searchDebounceMs?: number;
  loadRecentRecipients?: RecentRecipientSearchFunction;
  saveComposerDraft?: ComposerDraftSaveFunction;
  discardComposerDraft?: ComposerDraftDiscardFunction;
  stageComposerAttachment?: ComposerAttachmentStageFunction;
  stageForwardedAttachments?: ComposerForwardedAttachmentStageFunction;
  sendComposerMail?: ComposerSendFunction;
  discoverAccountSettings?: AccountDiscoveryFunction;
  createAccount?: AccountCreateFunction;
  reauthAccount?: AccountReauthFunction;
  startOAuthAccountSetup?: OAuthAccountSetupFunction;
  startOAuthAccountReauth?: OAuthAccountReauthFunction;
  removeAccount?: AccountRemoveFunction;
  updateAccountSettings?: AccountSettingsUpdateFunction;
  testAccountConnection?: AccountConnectionTestFunction;
  getAccountAuthStatus?: AccountAuthStatusFunction;
  exportDiagnosticsBundle?: DiagnosticsExportFunction;
  loadDiagnosticsHealth?: DiagnosticsHealthLoadFunction;
  subscribeToSyncStatus?: SyncStatusSubscribeFunction;
  onOpenSettings?: () => void;
};

type Drawer = "composer" | "drafts" | "settings" | "account-status" | null;
type ComposerRouteKind = "reply" | "reply-all" | "forward";
type MailSearchUiState = {
  query: string;
  results: MailMessage[];
  status: MailSearchUiStatus;
  parsedQuery: MailSearchParsedQuery | null;
  highlightTerms: string[];
};
type StoredSearches = {
  recent: string[];
  saved: string[];
};
type MailMessagePageScope = {
  accountId: string | null;
  folderId: string | null;
  unifiedInbox: boolean;
};
type AttachmentActionState = {
  attachmentId: string | null;
  status: "idle" | "loading" | "saved" | "downloaded" | "error";
  message: string | null;
};
type RemoteContentActionState = {
  messageId: string | null;
  status: "idle" | "loading" | "saved" | "error";
  message: string | null;
};
type ExternalLinkActionState = {
  linkId: string | null;
  status: "idle" | "opening" | "opened" | "error";
  message: string | null;
};
type ComposerRouteActionState = {
  status: "idle" | "loading" | "error";
  message: string | null;
};
type MailActionUiState = {
  status: "idle" | "running" | "succeeded" | "error";
  message: string | null;
  undo: MailActionUndo | null;
  retryRequest: MailActionRequest | null;
};
type MailActionConfirmation = {
  request: MailActionRequest;
  action: "delete-to-trash" | "expunge-deleted";
} | null;

const MESSAGE_PAGE_SIZE = 100;
const MAIL_SEARCH_DEBOUNCE_MS = 250;
const MAIL_SEARCH_RESULT_LIMIT = 100;
const MAIL_SEARCH_STORAGE_KEY = "komorebi.mail.searches.v1";
const MAIL_SEARCH_STORED_QUERY_LIMIT = 8;
const MAIL_SEARCH_STORED_QUERY_MAX_BYTES = 512;
const emptySearchState: MailSearchUiState = {
  highlightTerms: [],
  parsedQuery: null,
  query: "",
  results: [],
  status: "idle",
};
const emptyStoredSearches: StoredSearches = {
  recent: [],
  saved: [],
};
const emptyMessagePageState: MailMessagePageUiState = {
  cursor: null,
  hasMore: false,
  scopeKey: "",
  status: "idle",
  totalCount: null,
};
const emptyAttachmentActionState: AttachmentActionState = {
  attachmentId: null,
  status: "idle",
  message: null,
};
const emptyRemoteContentActionState: RemoteContentActionState = {
  messageId: null,
  status: "idle",
  message: null,
};
const emptyExternalLinkActionState: ExternalLinkActionState = {
  linkId: null,
  status: "idle",
  message: null,
};
const emptyComposerRouteActionState: ComposerRouteActionState = {
  status: "idle",
  message: null,
};
const emptyMailActionUiState: MailActionUiState = {
  status: "idle",
  message: null,
  undo: null,
  retryRequest: null,
};

const ComposerDrawer = lazy(() =>
  import("../composer/TipTapComposer").then((module) => ({
    default: module.TipTapComposer,
  })),
);

export function MailClientShell({
  initialData = emptyMailShellData,
  initialComposerDrafts = [],
  initialLoadState = readyMailShellLoadState,
  loadMailShellData,
  loadMessagePage,
  loadComposerDrafts,
  searchMail,
  allowRemoteContent,
  loadUiPreferences,
  updateUiPreferences,
  openExternalLink,
  performMailAction,
  previewAttachment,
  saveAttachment,
  downloadAttachment,
  searchDebounceMs = MAIL_SEARCH_DEBOUNCE_MS,
  loadRecentRecipients,
  saveComposerDraft,
  discardComposerDraft,
  stageComposerAttachment,
  stageForwardedAttachments,
  sendComposerMail,
  discoverAccountSettings,
  createAccount,
  reauthAccount,
  startOAuthAccountSetup,
  startOAuthAccountReauth,
  removeAccount,
  updateAccountSettings,
  testAccountConnection,
  getAccountAuthStatus,
  exportDiagnosticsBundle,
  loadDiagnosticsHealth,
  subscribeToSyncStatus,
  onOpenSettings,
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
  const [storedSearches, setStoredSearches] = useState<StoredSearches>(() =>
    readStoredSearches(),
  );
  const [messagePageState, setMessagePageState] =
    useState<MailMessagePageUiState>(emptyMessagePageState);
  const [messagePageRetryNonce, setMessagePageRetryNonce] = useState(0);
  const [composerDrafts, setComposerDrafts] =
    useState<ComposerDraft[]>(initialComposerDrafts);
  const [composerDraftsLoadState, setComposerDraftsLoadState] =
    useState<LoadStatus>(loadComposerDrafts ? "loading" : "ready");
  const [languagePreference, setLanguagePreference] = useState<
    UiPreferencesView["language"]
  >(null);
  const [languagePreferenceLoadState, setLanguagePreferenceLoadState] =
    useState<LoadStatus>(loadUiPreferences ? "loading" : "ready");
  const [drawer, setDrawer] = useState<Drawer>(null);
  const [statusAccountId, setStatusAccountId] = useState<string | null>(null);
  const [selectedComposerDraftId, setSelectedComposerDraftId] =
    useState<string | null>(null);
  const [routedComposerDraft, setRoutedComposerDraft] =
    useState<ComposerDraft | null>(null);
  const [attachmentPreview, setAttachmentPreview] =
    useState<AttachmentPreviewResponse | null>(null);
  const [attachmentAction, setAttachmentAction] =
    useState<AttachmentActionState>(emptyAttachmentActionState);
  const [remoteContentAction, setRemoteContentAction] =
    useState<RemoteContentActionState>(emptyRemoteContentActionState);
  const [externalLinkAction, setExternalLinkAction] =
    useState<ExternalLinkActionState>(emptyExternalLinkActionState);
  const [composerRouteAction, setComposerRouteAction] =
    useState<ComposerRouteActionState>(emptyComposerRouteActionState);
  const [mailAction, setMailAction] =
    useState<MailActionUiState>(emptyMailActionUiState);
  const [mailActionConfirmation, setMailActionConfirmation] =
    useState<MailActionConfirmation>(null);
  const accountButtonRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const folderButtonRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const readerRef = useRef<HTMLElement | null>(null);
  const composeButtonRef = useRef<HTMLButtonElement | null>(null);
  const draftsButtonRef = useRef<HTMLButtonElement | null>(null);

  const drawerRef = useRef<HTMLDivElement | null>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);
  const languagePreferenceRevisionRef = useRef(0);
  const visibleFolders = useMemo(() => foldersForScope(state), [state]);
  const scopedMessages = useMemo(() => messagesForScope(state), [state]);
  const searchQuery = state.searchQuery.trim();
  const isBackendSearchActive = Boolean(searchMail && searchQuery);
  const searchAccountId = state.unifiedInbox ? null : state.selectedAccountId;
  const searchFolderId = state.selectedFolderId;
  const messagePageScope = useMemo(
    () =>
      messagePageScopeForSelection(
        state.data.folders,
        state.selectedAccountId,
        state.selectedFolderId,
        state.unifiedInbox,
      ),
    [
      state.data.folders,
      state.selectedAccountId,
      state.selectedFolderId,
      state.unifiedInbox,
    ],
  );
  const messagePageScopeKey = messagePageScopeKeyForScope(messagePageScope);
  const activeMessagePageState =
    messagePageState.scopeKey === messagePageScopeKey
      ? messagePageState
      : emptyMessagePageState;
  const hasFreshSearchResults = searchState.query === searchQuery;
  const effectiveSearchStatus: MailSearchUiStatus = isBackendSearchActive
    ? hasFreshSearchResults
      ? searchState.status
      : "debouncing"
    : "idle";
  const effectiveSearchResults =
    isBackendSearchActive && hasFreshSearchResults
      ? searchState.results.filter((message) =>
          searchResultIsVisibleInScope(state, message),
        )
      : [];
  const effectiveSearchParsedQuery =
    isBackendSearchActive && hasFreshSearchResults
      ? searchState.parsedQuery
      : null;
  const effectiveSearchHighlightTerms =
    isBackendSearchActive && hasFreshSearchResults
      ? searchState.highlightTerms
      : [];
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
  const showAccountOnboarding =
    !hasAccounts && state.loadState.accounts === "ready";
  const canCompose = hasAccounts && state.loadState.accounts === "ready";
  const statusAccount = statusAccountId
    ? accountById.get(statusAccountId) ?? null
    : null;
  const selectedComposerDraft = selectedComposerDraftId
    ? composerDrafts.find((draft) => draft.id === selectedComposerDraftId) ?? null
    : null;
  const activeComposerDraft = selectedComposerDraft ?? routedComposerDraft;
  const listTitle = state.unifiedInbox
    ? t("mail.folders.unifiedInbox")
    : activeFolder?.name ?? activeAccount?.label ?? t("mail.folders.allMail");

  const saveLanguagePreference = useCallback(
    async (language: UiPreferencesView["language"]) => {
      if (!updateUiPreferences) {
        throw new Error(t("settings.language.unavailable"));
      }

      languagePreferenceRevisionRef.current += 1;
      const preferences = await updateUiPreferences({ language });
      setLanguagePreference(preferences.language);
      setLanguagePreferenceLoadState("ready");
      await komorebiI18n.changeLanguage(
        preferences.language ?? detectInitialLanguage(),
      );
    },
    [t, updateUiPreferences],
  );

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

  function openSettings(restoreFocus: HTMLElement | null) {
    if (onOpenSettings) {
      restoreFocusRef.current = restoreFocus;
      onOpenSettings();
      return;
    }

    openDrawer("settings", restoreFocus);
  }

  function closeDrawer() {
    const restoreFocus = restoreFocusRef.current;
    if (drawer === "composer") {
      setRoutedComposerDraft(null);
    }
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

  function updateStoredSearches(
    updater: (current: StoredSearches) => StoredSearches,
  ) {
    setStoredSearches((current) => {
      const next = normalizeStoredSearches(updater(current));
      writeStoredSearches(next);
      return next;
    });
  }

  function saveCurrentSearchQuery() {
    const normalizedQuery = normalizeStoredSearchQuery(state.searchQuery);
    if (!normalizedQuery) {
      return;
    }

    updateStoredSearches((current) => ({
      ...current,
      saved: addStoredQuery(current.saved, normalizedQuery),
    }));
  }

  function removeSavedSearchQuery(query: string) {
    const normalizedQuery = normalizeStoredSearchQuery(query);
    updateStoredSearches((current) => ({
      ...current,
      saved: current.saved.filter((stored) => stored !== normalizedQuery),
    }));
  }

  function clearSearchHistory() {
    updateStoredSearches((current) => ({
      ...current,
      recent: [],
    }));
  }

  function applyStoredSearchQuery(query: string) {
    clearAttachmentUi();
    setRemoteContentAction(emptyRemoteContentActionState);
    patchState({
      searchQuery: query,
      selectedMessageId: null,
    });
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

  async function reloadMailShell(options: { showLoading?: boolean } = {}) {
    if (!loadMailShellData) {
      return;
    }

    if (options.showLoading ?? true) {
      setState((current) => ({
        ...current,
        loadState: loadingMailShellLoadState(),
      }));
    }

    try {
      const data = await loadMailShellData();
      setState((current) =>
        mergeMailShellData(current, data, readyMailShellLoadState),
      );
      setMessagePageState(emptyMessagePageState);
    } catch {
      setState((current) => ({
        ...current,
        loadState: errorMailShellLoadState(),
      }));
    }
  }

  async function reloadComposerDrafts(options: { showLoading?: boolean } = {}) {
    if (!loadComposerDrafts) {
      return;
    }

    if (options.showLoading ?? true) {
      setComposerDraftsLoadState("loading");
    }

    try {
      const drafts = await loadComposerDrafts();
      setComposerDrafts(drafts);
      setComposerDraftsLoadState("ready");
      setSelectedComposerDraftId((current) =>
        current && drafts.some((draft) => draft.id === current)
          ? current
          : null,
      );
    } catch {
      setComposerDraftsLoadState("error");
    }
  }

  function clearAttachmentUi() {
    setAttachmentPreview(null);
    setAttachmentAction(emptyAttachmentActionState);
  }

  function clearReaderActionUi() {
    clearAttachmentUi();
    setRemoteContentAction(emptyRemoteContentActionState);
    setExternalLinkAction(emptyExternalLinkActionState);
    setComposerRouteAction(emptyComposerRouteActionState);
    setMailAction(emptyMailActionUiState);
    setMailActionConfirmation(null);
  }

  async function handlePreviewAttachment(
    message: MailMessage,
    attachment: MailAttachment,
  ) {
    if (!previewAttachment) {
      return;
    }

    setAttachmentAction({
      attachmentId: attachment.id,
      message: null,
      status: "loading",
    });

    try {
      const preview = await previewAttachment(
        attachmentRequestFor(message, attachment),
      );
      setAttachmentPreview(preview);
      setAttachmentAction(emptyAttachmentActionState);
    } catch {
      setAttachmentAction({
        attachmentId: attachment.id,
        message: t("mail.attachments.previewError"),
        status: "error",
      });
    }
  }

  async function handleSaveAttachment(
    message: MailMessage,
    attachment: MailAttachment,
  ) {
    if (!saveAttachment) {
      return;
    }

    setAttachmentAction({
      attachmentId: attachment.id,
      message: null,
      status: "loading",
    });

    try {
      const saved = await saveAttachment(attachmentRequestFor(message, attachment));
      setAttachmentAction({
        attachmentId: attachment.id,
        message: t("mail.attachments.saveSuccess", {
          filename: saved.savedFilename,
          destination: saved.destinationLabel,
        }),
        status: "saved",
      });
    } catch {
      setAttachmentAction({
        attachmentId: attachment.id,
        message: t("mail.attachments.saveError"),
        status: "error",
      });
    }
  }

  async function handleDownloadAttachment(
    message: MailMessage,
    attachment: MailAttachment,
  ) {
    if (!downloadAttachment) {
      return;
    }

    setAttachmentAction({
      attachmentId: attachment.id,
      message: null,
      status: "loading",
    });

    try {
      const download = await downloadAttachment(
        attachmentRequestFor(message, attachment),
      );
      triggerBrowserDownload(download);
      setAttachmentAction({
        attachmentId: attachment.id,
        message: t("mail.attachments.downloadStarted", {
          filename: download.displayFilename,
        }),
        status: "downloaded",
      });
    } catch {
      setAttachmentAction({
        attachmentId: attachment.id,
        message: t("mail.attachments.downloadError"),
        status: "error",
      });
    }
  }

  async function handleAllowRemoteContent(
    message: MailMessage,
    scope: RemoteContentPreferenceScope,
  ) {
    if (!allowRemoteContent) {
      return;
    }

    setRemoteContentAction({
      message: null,
      messageId: message.id,
      status: "loading",
    });

    try {
      const response = await allowRemoteContent({
        accountId: message.accountId,
        folderId: message.folderId,
        messageId: message.id,
        scope,
        userRequested: true,
      });
      applyRemoteContentPreferenceResponse(response);
      setRemoteContentAction({
        message: t(
          scope === "sender"
            ? "mail.reader.remoteContent.allowedSenderStatus"
            : "mail.reader.remoteContent.allowedMessageStatus",
        ),
        messageId: message.id,
        status: "saved",
      });
      if (loadMailShellData) {
        await reloadMailShell({ showLoading: false });
      }
    } catch {
      setRemoteContentAction({
        message: t("mail.reader.remoteContent.error"),
        messageId: message.id,
        status: "error",
      });
    }
  }

  async function handleOpenExternalLink(link: SafeEmailLink) {
    if (!openExternalLink) {
      setExternalLinkAction({
        linkId: link.id,
        message: t("mail.reader.links.unavailable"),
        status: "error",
      });
      return;
    }

    setExternalLinkAction({
      linkId: link.id,
      message: t("mail.reader.links.opening"),
      status: "opening",
    });

    try {
      await openExternalLink({
        url: link.url,
        userRequested: true,
      });
      setExternalLinkAction({
        linkId: link.id,
        message: t("mail.reader.links.opened"),
        status: "opened",
      });
    } catch {
      setExternalLinkAction({
        linkId: link.id,
        message: t("mail.reader.links.error"),
        status: "error",
      });
    }
  }

  function requestMailAction(request: MailActionRequest) {
    if (
      !request.confirmed &&
      (request.action.kind === "delete-to-trash" ||
        request.action.kind === "expunge-deleted")
    ) {
      setMailActionConfirmation({
        action: request.action.kind,
        request: {
          ...request,
          confirmed: true,
        },
      });
      setMailAction(emptyMailActionUiState);
      return;
    }

    void executeMailAction(request);
  }

  async function executeMailAction(request: MailActionRequest) {
    if (!performMailAction) {
      setMailAction({
        message: t("mail.reader.mailActions.unavailable"),
        retryRequest: null,
        status: "error",
        undo: null,
      });
      return;
    }

    setMailActionConfirmation(null);
    setMailAction({
      message: mailActionRunningMessage(request.action, t),
      retryRequest: null,
      status: "running",
      undo: null,
    });

    try {
      const response = await performMailAction(request);
      applyMailActionResponse(response);
      setMailAction({
        message: mailActionSuccessMessage(response, t),
        retryRequest: null,
        status: "succeeded",
        undo: response.undo,
      });
      if (loadMailShellData) {
        await reloadMailShell({ showLoading: false });
      }
    } catch (error) {
      const retry = retryDecisionFromError(error);
      setMailAction({
        message: mailActionFailureMessage(error, t),
        retryRequest:
          retry?.kind === "retry-now"
            ? {
                ...request,
                attempt: retry.nextAttempt,
              }
            : null,
        status: "error",
        undo: null,
      });
    }
  }

  function applyMailActionResponse(response: MailActionResponse) {
    setState((current) => {
      const messages = current.data.messages.map((message) => {
        if (response.mutation.kind === "flags-updated") {
          return message.accountId === response.accountId &&
            message.id === messageIdFromActionResponse(response)
            ? { ...message, unread: !response.mutation.read }
            : message;
        }

        if (
          response.mutation.kind === "moved" &&
          message.accountId === response.accountId &&
          message.folderId === response.mutation.fromFolderId &&
          response.undo?.messageId === message.id
        ) {
          return {
            ...message,
            folderId: response.mutation.toFolderId,
          };
        }

        return message;
      });
      const folders = foldersAfterMailAction(
        current.data.folders,
        current.data.messages,
        response,
      );

      return {
        ...current,
        data: {
          ...current.data,
          accounts: accountsWithFolderUnreadCounts(current.data.accounts, folders),
          folders,
          messages,
        },
      };
    });
  }

  function openNewComposer(restoreFocus: HTMLElement | null) {
    setSelectedComposerDraftId(null);
    setRoutedComposerDraft(null);
    setComposerRouteAction(emptyComposerRouteActionState);
    openDrawer("composer", restoreFocus);
  }

  function openComposerDraft(
    draftId: string,
    restoreFocus: HTMLElement | null = draftsButtonRef.current,
  ) {
    setSelectedComposerDraftId(draftId);
    setRoutedComposerDraft(null);
    setComposerRouteAction(emptyComposerRouteActionState);
    openDrawer("composer", restoreFocus);
  }

  async function openRoutedComposer(
    message: MailMessage,
    route: ComposerRouteKind,
    includeAttachments: boolean,
    restoreFocus: HTMLElement | null,
  ) {
    if (!canCompose) {
      return;
    }

    const draftId = createRoutedDraftId(route, message.id);
    let attachments: ComposerAttachment[] = [];
    if (includeAttachments && message.attachments.length > 0) {
      if (!stageForwardedAttachments) {
        setComposerRouteAction({
          message: t("mail.reader.routing.forwardAttachmentUnavailable"),
          status: "error",
        });
        return;
      }

      setComposerRouteAction({
        message: t("mail.reader.routing.stagingAttachments"),
        status: "loading",
      });

      try {
        attachments = await stageForwardedAttachments({
          accountId: message.accountId,
          draftId,
          sourceAttachmentIds: message.attachments.map(
            (attachment) => attachment.id,
          ),
          sourceFolderId: message.folderId,
          sourceMessageId: message.id,
          userRequested: true,
        });
      } catch {
        setComposerRouteAction({
          message: t("mail.reader.routing.forwardAttachmentError"),
          status: "error",
        });
        return;
      }
    }

    const sourceAccount = accountById.get(message.accountId) ?? null;
    setSelectedComposerDraftId(null);
    setRoutedComposerDraft(
      composerDraftFromMessage({
        attachments,
        draftId,
        localeFormat,
        message,
        route,
        sourceAccount,
        t,
      }),
    );
    setComposerRouteAction(emptyComposerRouteActionState);
    openDrawer("composer", restoreFocus);
  }

  function handleComposerDraftChanged() {
    if (loadComposerDrafts) {
      void reloadComposerDrafts({ showLoading: false });
    }
  }

  function handleComposerDraftClosed(draftId: string) {
    setComposerDrafts((current) =>
      current.filter((draft) => draft.id !== draftId),
    );
    setSelectedComposerDraftId((current) => (current === draftId ? null : current));
    if (loadComposerDrafts) {
      void reloadComposerDrafts({ showLoading: false });
    }
  }

  function openAccountStatusDrawer(
    accountId: string,
    restoreFocus: HTMLElement | null,
  ) {
    setStatusAccountId(accountId);
    openDrawer("account-status", restoreFocus);
  }

  async function saveAccountSettings(
    request: AccountSettingsUpdateRequest,
  ): Promise<AccountSettingsUpdateResponse> {
    if (updateAccountSettings) {
      const response = await updateAccountSettings(request);
      if (loadMailShellData) {
        await reloadMailShell();
      } else {
        applyAccountSettingsResponse(response);
      }
      return response;
    }

    const account = state.data.accounts.find(
      (candidate) => candidate.id === request.accountId,
    );
    if (!account) {
      throw new Error(t("settings.accounts.errors.settingsUnavailable"));
    }

    const response: AccountSettingsUpdateResponse = {
      account: {
        id: account.id,
        providerKind: account.providerKind ?? "generic_imap_smtp",
        displayEmail: account.email,
        displayName: request.displayName?.trim() || null,
        username: account.email,
        authKind: account.authKind ?? "manual_password",
        authStatus: authStatusForManagement(account.status),
      },
      signatureHtml: request.signatureHtml.trim() || null,
      localPreferences: request.localPreferences,
    };

    applyAccountSettingsResponse(response);

    return response;
  }

  function applyAccountSettingsResponse(
    response: AccountSettingsUpdateResponse,
  ) {
    setState((current) => ({
      ...current,
      data: {
        ...current.data,
        accounts: current.data.accounts.map((currentAccount) =>
          currentAccount.id === response.account.id
            ? {
                ...currentAccount,
                displayName: response.account.displayName,
                label:
                  response.account.displayName?.trim() ||
                  response.account.displayEmail,
                signatureHtml: response.signatureHtml,
                localPreferences: response.localPreferences,
              }
            : currentAccount,
        ),
      },
    }));
  }

  function applyRemoteContentPreferenceResponse(
    response: RemoteContentPreferenceResponse,
  ) {
    const patchMessage = (message: MailMessage): MailMessage =>
      message.id === response.messageId &&
      message.accountId === response.accountId
        ? {
            ...message,
            remoteContentAllowed: response.remoteContentAllowed,
            remoteContentAllowedScope: response.scope,
          }
        : message;

    setState((current) => ({
      ...current,
      data: {
        ...current.data,
        messages: current.data.messages.map(patchMessage),
      },
    }));
    setSearchState((current) => ({
      ...current,
      results: current.results.map(patchMessage),
    }));
  }

  useEffect(() => {
    if (!loadUiPreferences) {
      return;
    }

    let cancelled = false;
    const revision = languagePreferenceRevisionRef.current;

    loadUiPreferences()
      .then((preferences) => {
        if (
          cancelled ||
          revision !== languagePreferenceRevisionRef.current
        ) {
          return;
        }

        setLanguagePreference(preferences.language);
        setLanguagePreferenceLoadState("ready");
        void komorebiI18n.changeLanguage(
          preferences.language ?? detectInitialLanguage(),
        );
      })
      .catch(() => {
        if (cancelled || revision !== languagePreferenceRevisionRef.current) {
          return;
        }
        setLanguagePreferenceLoadState("error");
      });

    return () => {
      cancelled = true;
    };
  }, [loadUiPreferences]);

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
        setMessagePageState(emptyMessagePageState);
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
    if (!loadComposerDrafts) {
      return;
    }

    let cancelled = false;

    loadComposerDrafts()
      .then((drafts) => {
        if (cancelled) {
          return;
        }
        setComposerDrafts(drafts);
        setComposerDraftsLoadState("ready");
      })
      .catch(() => {
        if (cancelled) {
          return;
        }
        setComposerDraftsLoadState("error");
      });

    return () => {
      cancelled = true;
    };
  }, [loadComposerDrafts]);

  useEffect(() => {
    if (!subscribeToSyncStatus || !loadMailShellData) {
      return;
    }

    let cancelled = false;
    let reloadHandle: number | null = null;
    let unsubscribe: SyncStatusUnsubscribeFunction | null = null;

    const refreshFromEvent = () => {
      if (reloadHandle !== null) {
        window.clearTimeout(reloadHandle);
      }
      reloadHandle = window.setTimeout(() => {
        reloadHandle = null;
        loadMailShellData()
          .then((data) => {
            if (cancelled) {
              return;
            }
            setState((current) =>
              mergeMailShellData(current, data, readyMailShellLoadState),
            );
            setMessagePageState(emptyMessagePageState);
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
      }, 75);
    };

    Promise.resolve(subscribeToSyncStatus(refreshFromEvent))
      .then((nextUnsubscribe) => {
        if (cancelled) {
          void nextUnsubscribe();
          return;
        }
        unsubscribe = nextUnsubscribe;
      })
      .catch(() => undefined);

    return () => {
      cancelled = true;
      if (reloadHandle !== null) {
        window.clearTimeout(reloadHandle);
      }
      if (unsubscribe) {
        void unsubscribe();
      }
    };
  }, [loadMailShellData, subscribeToSyncStatus]);

  useEffect(() => {
    if (
      !loadMessagePage ||
      !hasAccounts ||
      searchQuery ||
      state.loadState.messages !== "ready"
    ) {
      return;
    }

    const scope = messagePageScope;
    const scopeKey = messagePageScopeKey;
    const controller = new AbortController();
    queueMicrotask(() => {
      if (controller.signal.aborted) {
        return;
      }

      setMessagePageState({
        cursor: null,
        hasMore: false,
        scopeKey,
        status: "loading",
        totalCount: null,
      });

      loadMessagePage(
        messagePageRequestForScope(scope, null, MESSAGE_PAGE_SIZE),
        controller.signal,
      )
        .then((response) => {
          if (controller.signal.aborted) {
            return;
          }

          setState((current) =>
            currentMessagePageScopeKey(current) === scopeKey
              ? mergeMessagePageIntoState(
                  current,
                  scope,
                  response.messages,
                  "replace",
                )
              : current,
          );
          setMessagePageState((current) =>
            current.scopeKey === scopeKey
              ? {
                  cursor: response.nextCursor,
                  hasMore: response.hasMore,
                  scopeKey,
                  status: "idle",
                  totalCount: response.totalCount,
                }
              : current,
          );
        })
        .catch((error: unknown) => {
          if (controller.signal.aborted || isAbortError(error)) {
            return;
          }

          setMessagePageState((current) =>
            current.scopeKey === scopeKey
              ? {
                  ...current,
                  hasMore: false,
                  status: "error",
                }
              : current,
          );
        });
    });

    return () => {
      controller.abort();
    };
  }, [
    hasAccounts,
    loadMessagePage,
    messagePageRetryNonce,
    messagePageScope,
    messagePageScopeKey,
    searchQuery,
    state.loadState.messages,
  ]);

  const loadNextMessagePage = useCallback(() => {
    if (
      !loadMessagePage ||
      !hasAccounts ||
      searchQuery ||
      state.loadState.messages !== "ready" ||
      activeMessagePageState.status === "loading" ||
      !activeMessagePageState.hasMore ||
      !activeMessagePageState.cursor
    ) {
      return;
    }

    const scope = messagePageScope;
    const scopeKey = messagePageScopeKey;
    const cursor = activeMessagePageState.cursor;
    const controller = new AbortController();
    setMessagePageState((current) =>
      current.scopeKey === scopeKey
        ? {
            ...current,
            status: "loading",
          }
        : current,
    );

    loadMessagePage(
      messagePageRequestForScope(scope, cursor, MESSAGE_PAGE_SIZE),
      controller.signal,
    )
      .then((response) => {
        if (controller.signal.aborted) {
          return;
        }

        setState((current) =>
          currentMessagePageScopeKey(current) === scopeKey
            ? mergeMessagePageIntoState(current, scope, response.messages, "append")
            : current,
        );
        setMessagePageState((current) =>
          current.scopeKey === scopeKey
            ? {
                cursor: response.nextCursor,
                hasMore: response.hasMore,
                scopeKey,
                status: "idle",
                totalCount: response.totalCount,
              }
            : current,
        );
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted || isAbortError(error)) {
          return;
        }

        setMessagePageState((current) =>
          current.scopeKey === scopeKey
            ? {
                ...current,
                status: "error",
              }
            : current,
        );
      });
  }, [
    activeMessagePageState.cursor,
    activeMessagePageState.hasMore,
    activeMessagePageState.status,
    hasAccounts,
    loadMessagePage,
    messagePageScope,
    messagePageScopeKey,
    searchQuery,
    state.loadState.messages,
  ]);

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
        highlightTerms: current.query === searchQuery ? current.highlightTerms : [],
        parsedQuery: current.query === searchQuery ? current.parsedQuery : null,
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
            highlightTerms: response.highlightTerms ?? [],
            parsedQuery: response.parsedQuery ?? null,
            query: response.query,
            results: response.results,
            status:
              response.indexStatus === "unavailable" ? "unavailable" : "ready",
          });
          if (response.indexStatus === "ready") {
            setStoredSearches((current) => {
              const normalizedQuery = normalizeStoredSearchQuery(response.query);
              if (!normalizedQuery) {
                return current;
              }
              const next = normalizeStoredSearches({
                ...current,
                recent: addStoredQuery(current.recent, normalizedQuery),
              });
              writeStoredSearches(next);
              return next;
            });
          }
        })
        .catch((error: unknown) => {
          if (controller.signal.aborted || isAbortError(error)) {
            return;
          }

          setSearchState({
            highlightTerms: [],
            parsedQuery: null,
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
    <main className="app-frame" onKeyDown={handleShellKeyDown}>
      <MailClientTitleBar
        scopeLabel={
          state.unifiedInbox
            ? t("app.scope.allAccounts")
            : activeAccount?.label ?? t("app.scope.local")
        }
        title={listTitle}
      />
      <section className="app-shell">
      <MailSidebar
        state={state}
        visibleFolders={visibleFolders}
        onAccountButtonRef={(index, element) => {
          accountButtonRefs.current[index] = element;
        }}
        onAccountKeyDown={handleAccountKeyDown}
        onFolderButtonRef={(index, element) => {
          folderButtonRefs.current[index] = element;
        }}
        onFolderKeyDown={handleFolderKeyDown}
        onOpenAccountStatus={openAccountStatusDrawer}
        onOpenSettings={openSettings}
        onRetry={loadMailShellData ? reloadMailShell : undefined}
        onSelectAccount={(accountId) =>
          setState((current) => selectAccountScope(current, accountId))
        }
        onSelectFolder={(folderId) =>
          setState((current) => selectFolderScope(current, folderId))
        }
        onSelectUnified={(unified) =>
          setState((current) =>
            unified
              ? selectUnifiedScope(current)
              : selectAccountScope(
                  current,
                  current.selectedAccountId ?? current.data.accounts[0]?.id ?? "",
                ),
          )
        }
      />

      {showAccountOnboarding ? (
        <section className="mail-onboarding" aria-label={t("mail.onboarding.label")}>
          <div className="mail-onboarding__panel">
            <span className="mail-onboarding__mark" aria-hidden="true">
              <Leaf size={28} />
            </span>
            <p className="eyebrow">{t("mail.onboarding.eyebrow")}</p>
            <h2>{t("mail.onboarding.heading")}</h2>
            <p>{t("mail.onboarding.body")}</p>
            <div className="mail-onboarding__actions">
              <button type="button" onClick={() => openSettings(null)}>
                <Plus aria-hidden="true" size={16} />
                {t("mail.account.add")}
              </button>
              <span>{t("mail.onboarding.secondary")}</span>
            </div>
          </div>
        </section>
      ) : (
        <>
      <section className="message-list" aria-label={t("app.sections.messages")}>
        <header className="list-toolbar">
          <div>
            <p className="eyebrow">
              {state.unifiedInbox ? t("app.scope.allAccounts") : activeAccount?.label ?? t("app.scope.local")}
            </p>
            <h2>{listTitle}</h2>
          </div>
          <div className="list-toolbar__actions">
            <button
              ref={draftsButtonRef}
              type="button"
              disabled={
                !canCompose ||
                (composerDraftsLoadState === "ready" && composerDrafts.length === 0)
              }
              aria-label={t("composer.drafts.buttonAria", {
                count: composerDrafts.length,
                formattedCount: localeFormat.count(composerDrafts.length),
              })}
              onClick={() => openDrawer("drafts", draftsButtonRef.current)}
            >
              {t("composer.drafts.button")}
              <span>{localeFormat.count(composerDrafts.length)}</span>
            </button>
            <button
              ref={composeButtonRef}
              type="button"
              disabled={!canCompose}
              onClick={() => openNewComposer(composeButtonRef.current)}
            >
              {t("composer.actions.compose")}
            </button>
          </div>
        </header>

        <div className="search">
          <label htmlFor="mail-search-input">{t("mail.search.label")}</label>
          <input
            id="mail-search-input"
            ref={searchInputRef}
            type="search"
            placeholder={t("mail.search.placeholder")}
            disabled={!hasAccounts || state.loadState.messages !== "ready"}
            value={state.searchQuery}
            onChange={(event) => {
              clearAttachmentUi();
              setRemoteContentAction(emptyRemoteContentActionState);
              patchState({
                searchQuery: event.target.value,
                selectedMessageId: null,
              });
            }}
          />
          <SearchStatusLine
            isBackendSearchActive={isBackendSearchActive}
            resultCount={visibleMessages.length}
            status={effectiveSearchStatus}
          />
          <SearchQueryTools
            currentQuery={searchQuery}
            parsedQuery={effectiveSearchParsedQuery}
            recentSearches={storedSearches.recent}
            savedSearches={storedSearches.saved}
            onApplySearch={applyStoredSearchQuery}
            onClearHistory={clearSearchHistory}
            onRemoveSavedSearch={removeSavedSearchQuery}
            onSaveSearch={saveCurrentSearchQuery}
          />
        </div>

        <MessageListPane
          accountById={accountById}
          folderById={folderById}
          isBackendSearchActive={isBackendSearchActive}
          state={state}
          searchStatus={effectiveSearchStatus}
          paginationStatus={activeMessagePageState}
          visibleMessages={visibleMessages}
          onLoadMoreMessages={
            loadMessagePage && !isBackendSearchActive
              ? loadNextMessagePage
              : undefined
          }
          onRetry={loadMailShellData ? reloadMailShell : undefined}
          onRetryPage={() => setMessagePageRetryNonce((current) => current + 1)}
          onRetrySearch={() => setSearchRetryNonce((current) => current + 1)}
          searchHighlightTerms={effectiveSearchHighlightTerms}
          onSelectMessage={(messageId) => {
            clearReaderActionUi();
            patchState({
              selectedMessageId: messageId,
            });
          }}
          onReadMessage={(messageId) => {
            clearReaderActionUi();
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
          key={activeMessage?.id ?? "empty-reader"}
          state={state}
          message={activeMessage}
          hasVisibleMessages={visibleMessages.length > 0}
          canCompose={canCompose}
          canForwardAttachments={Boolean(stageForwardedAttachments)}
          onRetry={loadMailShellData ? reloadMailShell : undefined}
          previewAttachment={previewAttachment}
          saveAttachment={saveAttachment}
          downloadAttachment={downloadAttachment}
          allowRemoteContent={allowRemoteContent}
          attachmentPreview={attachmentPreview}
          attachmentAction={attachmentAction}
          remoteContentAction={remoteContentAction}
          externalLinkAction={externalLinkAction}
          composerRouteAction={composerRouteAction}
          mailAction={mailAction}
          mailActionConfirmation={mailActionConfirmation}
          performMailAction={performMailAction}
          onPreviewAttachment={handlePreviewAttachment}
          onSaveAttachment={handleSaveAttachment}
          onDownloadAttachment={handleDownloadAttachment}
          onAllowRemoteContent={handleAllowRemoteContent}
          onOpenExternalLink={handleOpenExternalLink}
          onRouteComposer={(message, route, includeAttachments, restoreFocus) => {
            void openRoutedComposer(
              message,
              route,
              includeAttachments,
              restoreFocus,
            );
          }}
          onRequestMailAction={requestMailAction}
          onConfirmMailAction={() => {
            if (mailActionConfirmation) {
              void executeMailAction(mailActionConfirmation.request);
            }
          }}
          onCancelMailAction={() => setMailActionConfirmation(null)}
          onRetryMailAction={() => {
            if (mailAction.retryRequest) {
              void executeMailAction(mailAction.retryRequest);
            }
          }}
          onUndoMailAction={() => {
            if (mailAction.undo) {
              void executeMailAction(mailActionRequestFromUndo(mailAction.undo));
            }
          }}
          onCloseAttachmentPreview={() => setAttachmentPreview(null)}
        />
      </section>
        </>
      )}

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
              displayName: account.displayName ?? null,
              providerKind: account.providerKind,
              authKind: account.authKind,
              signatureHtml: account.signatureHtml,
              localPreferences: accountLocalPreferences(account),
            }))}
            initialAccountId={state.selectedAccountId}
            languagePreference={languagePreference}
            languagePreferenceStatus={languagePreferenceLoadState}
            onChangeLanguagePreference={
              updateUiPreferences ? saveLanguagePreference : undefined
            }
            onSaveAccountSettings={saveAccountSettings}
            onDiscoverAccountSettings={discoverAccountSettings}
            onAccountChanged={() => {
              if (loadMailShellData) {
                void reloadMailShell();
              }
              if (loadComposerDrafts) {
                void reloadComposerDrafts({ showLoading: false });
              }
            }}
            onCreateAccount={createAccount}
            onReauthAccount={reauthAccount}
            onStartOAuthAccountSetup={startOAuthAccountSetup}
            onStartOAuthAccountReauth={startOAuthAccountReauth}
            onRemoveAccount={removeAccount}
            onTestAccountConnection={testAccountConnection}
            onGetAccountAuthStatus={getAccountAuthStatus}
            onExportDiagnostics={exportDiagnosticsBundle}
            onLoadDiagnosticsHealth={loadDiagnosticsHealth}
            onClose={closeDrawer}
          />
        </div>
      ) : null}

      {drawer === "drafts" ? (
        <div
          className="composer-drawer"
          role="dialog"
          aria-modal="true"
          aria-label={t("composer.drafts.aria")}
          ref={drawerRef}
          tabIndex={-1}
          onKeyDown={handleDrawerKeyDown}
        >
          <DraftsDrawer
            accounts={state.data.accounts}
            drafts={composerDrafts}
            loadState={composerDraftsLoadState}
            onClose={closeDrawer}
            onOpenDraft={(draftId) => openComposerDraft(draftId)}
            onRetry={
              loadComposerDrafts
                ? () => void reloadComposerDrafts()
                : undefined
            }
          />
        </div>
      ) : null}

      {drawer === "account-status" && statusAccount ? (
        <div
          className="settings-drawer"
          role="dialog"
          aria-modal="true"
          aria-label={t("mail.account.statusDrawer.aria")}
          ref={drawerRef}
          tabIndex={-1}
          onKeyDown={handleDrawerKeyDown}
        >
          <AccountStatusDrawer
            account={statusAccount}
            localeFormat={localeFormat}
            onClose={closeDrawer}
            onReconnect={async (secret) => {
              if (
                statusAccount.authKind === "oauth2" &&
                statusAccount.providerKind !== "generic_imap_smtp"
              ) {
                if (!startOAuthAccountReauth) {
                  throw new Error(
                    t("settings.accounts.errors.oauthReauthUnavailable"),
                  );
                }
                await startOAuthAccountReauth({
                  accountId: statusAccount.id,
                  clientId: null,
                });
              } else {
                if (!reauthAccount) {
                  throw new Error(t("settings.accounts.errors.reauthUnavailable"));
                }
                await reauthAccount({
                  accountId: statusAccount.id,
                  secret,
                });
              }

              if (loadMailShellData) {
                await reloadMailShell({ showLoading: false });
              }
            }}
            requiresSecret={
              statusAccount.authKind !== "oauth2" ||
              statusAccount.providerKind === "generic_imap_smtp"
            }
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
          <Suspense fallback={<div className="composer-loading" />}>
            <ComposerDrawer
              accounts={state.data.accounts}
              discardDraft={discardComposerDraft}
              initialDraft={activeComposerDraft}
              initialAccountId={
                activeAccount?.id ?? state.data.accounts[0]?.id ?? null
              }
              loadRecentRecipients={loadRecentRecipients}
              onClose={closeDrawer}
              onDraftDiscarded={handleComposerDraftClosed}
              onDraftSaved={handleComposerDraftChanged}
              onDraftSent={handleComposerDraftClosed}
              saveDraft={saveComposerDraft}
              sendMail={sendComposerMail}
              stageAttachment={stageComposerAttachment}
            />
          </Suspense>
        </div>
      ) : null}
      </section>
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

function messagePageScopeForSelection(
  folders: MailShellState["data"]["folders"],
  selectedAccountId: string | null,
  selectedFolderId: string | null,
  unifiedInbox: boolean,
): MailMessagePageScope {
  if (selectedFolderId) {
    const folder = folders.find(
      (candidate) => candidate.id === selectedFolderId,
    );
    return {
      accountId: folder?.accountId ?? selectedAccountId,
      folderId: selectedFolderId,
      unifiedInbox: false,
    };
  }

  if (unifiedInbox) {
    return {
      accountId: null,
      folderId: null,
      unifiedInbox: true,
    };
  }

  return {
    accountId: selectedAccountId,
    folderId: null,
    unifiedInbox: false,
  };
}

function messagePageScopeForState(state: MailShellState): MailMessagePageScope {
  return messagePageScopeForSelection(
    state.data.folders,
    state.selectedAccountId,
    state.selectedFolderId,
    state.unifiedInbox,
  );
}

function messagePageScopeKeyForScope(scope: MailMessagePageScope) {
  if (scope.unifiedInbox) {
    return "unified";
  }

  return `account:${scope.accountId ?? "none"}:folder:${scope.folderId ?? "all"}`;
}

function currentMessagePageScopeKey(state: MailShellState) {
  return messagePageScopeKeyForScope(messagePageScopeForState(state));
}

function messagePageRequestForScope(
  scope: MailMessagePageScope,
  cursor: MailMessagePageCursor | null,
  limit: number,
): MailMessagePageRequest {
  return {
    accountId: scope.accountId,
    cursor,
    folderId: scope.folderId,
    limit,
    unifiedInbox: scope.unifiedInbox,
  };
}

function mergeMessagePageIntoState(
  state: MailShellState,
  scope: MailMessagePageScope,
  messages: MailMessage[],
  mode: "replace" | "append",
): MailShellState {
  const incomingIds = new Set(messages.map((message) => message.id));
  const retainedMessages = state.data.messages
    .filter((message) => !incomingIds.has(message.id))
    .filter((message) =>
      mode === "append" ? true : !messageBelongsToPageScope(state, scope, message),
    );

  return mergeMailShellData(
    state,
    {
      ...state.data,
      messages: [...retainedMessages, ...messages],
    },
    state.loadState,
  );
}

function messageBelongsToPageScope(
  state: MailShellState,
  scope: MailMessagePageScope,
  message: MailMessage,
) {
  if (scope.folderId) {
    return (
      message.folderId === scope.folderId &&
      (!scope.accountId || message.accountId === scope.accountId)
    );
  }

  if (scope.unifiedInbox) {
    const account = state.data.accounts.find(
      (candidate) => candidate.id === message.accountId,
    );
    const folder = state.data.folders.find(
      (candidate) =>
        candidate.id === message.folderId &&
        candidate.accountId === message.accountId,
    );

    return (
      Boolean(account) &&
      Boolean(folder) &&
      accountLocalPreferences(account!).includeInUnifiedInbox &&
      folder!.kind === "inbox"
    );
  }

  return Boolean(scope.accountId) && message.accountId === scope.accountId;
}

function isAbortError(error: unknown) {
  return error instanceof DOMException && error.name === "AbortError";
}

function readStoredSearches(): StoredSearches {
  if (typeof window === "undefined") {
    return emptyStoredSearches;
  }

  try {
    const rawValue = window.localStorage.getItem(MAIL_SEARCH_STORAGE_KEY);
    if (!rawValue) {
      return emptyStoredSearches;
    }
    const parsedValue = JSON.parse(rawValue) as Partial<StoredSearches>;
    return normalizeStoredSearches({
      recent: Array.isArray(parsedValue.recent) ? parsedValue.recent : [],
      saved: Array.isArray(parsedValue.saved) ? parsedValue.saved : [],
    });
  } catch {
    return emptyStoredSearches;
  }
}

function writeStoredSearches(searches: StoredSearches) {
  if (typeof window === "undefined") {
    return;
  }

  try {
    window.localStorage.setItem(
      MAIL_SEARCH_STORAGE_KEY,
      JSON.stringify(normalizeStoredSearches(searches)),
    );
  } catch {
    // Search history is a convenience cache; quota or storage policy failures
    // must not block local mail search.
  }
}

function normalizeStoredSearches(searches: StoredSearches): StoredSearches {
  return {
    recent: normalizeStoredSearchList(searches.recent),
    saved: normalizeStoredSearchList(searches.saved),
  };
}

function normalizeStoredSearchList(queries: string[]) {
  const normalized: string[] = [];
  for (const query of queries) {
    const value = normalizeStoredSearchQuery(query);
    if (value && !normalized.includes(value)) {
      normalized.push(value);
    }
    if (normalized.length >= MAIL_SEARCH_STORED_QUERY_LIMIT) {
      break;
    }
  }

  return normalized;
}

function addStoredQuery(queries: string[], query: string) {
  return normalizeStoredSearchList([
    query,
    ...queries.filter((storedQuery) => storedQuery !== query),
  ]);
}

function normalizeStoredSearchQuery(query: string) {
  const normalized = query.trim().replace(/\s+/g, " ");
  if (
    !normalized ||
    new TextEncoder().encode(normalized).length >
      MAIL_SEARCH_STORED_QUERY_MAX_BYTES
  ) {
    return "";
  }

  return normalized;
}

function attachmentRequestFor(
  message: MailMessage,
  attachment: MailAttachment,
): AttachmentCommandRequest {
  return {
    accountId: message.accountId,
    folderId: message.folderId,
    messageId: message.id,
    attachmentId: attachment.id,
    userRequested: true,
  };
}

function mailActionRequestFromUndo(undo: MailActionUndo): MailActionRequest {
  return {
    accountId: undo.accountId,
    action: undo.action,
    confirmed: false,
    folderId: undo.folderId,
    messageId: undo.messageId,
    userRequested: true,
  };
}

function composerDraftFromMessage({
  attachments,
  draftId,
  localeFormat,
  message,
  route,
  sourceAccount,
  t,
}: {
  attachments: ComposerAttachment[];
  draftId: string;
  localeFormat: ReturnType<typeof useLocaleFormatters>;
  message: MailMessage;
  route: ComposerRouteKind;
  sourceAccount: MailAccount | null;
  t: TFunction;
}): ComposerDraft {
  const quoted = quotedBodyFromMessage(message, route, localeFormat, t);
  const recipients = composerRecipientsForRoute(message, route, sourceAccount);
  const references =
    route === "reply" || route === "reply-all"
      ? replyReferencesForMessage(message)
      : [];

  return {
    accountId: message.accountId,
    attachments,
    autosaveRevision: 0,
    bcc: [],
    bodyHtml: quoted.bodyHtml,
    bodyText: quoted.bodyText,
    cc: recipients.cc,
    id: draftId,
    inReplyTo:
      route === "reply" || route === "reply-all"
        ? (message.rfcMessageId ?? null)
        : null,
    references,
    subject: subjectForRoute(message.subject, route, t),
    to: recipients.to,
  };
}

function composerRecipientsForRoute(
  message: MailMessage,
  route: ComposerRouteKind,
  sourceAccount: MailAccount | null,
): { to: ComposerDraft["to"]; cc: ComposerDraft["cc"] } {
  if (route === "forward") {
    return { cc: [], to: [] };
  }

  const ownEmail = sourceAccount?.email.trim().toLocaleLowerCase() ?? "";
  const sender = composerAddressFromSender(message);
  const senderEmail = sender?.address.trim().toLocaleLowerCase() ?? "";
  const to = sender && senderEmail !== ownEmail ? [sender] : [];
  if (route === "reply") {
    return { cc: [], to };
  }

  const cc = dedupeComposerAddresses(
    [...(message.toAddresses ?? []), ...(message.ccAddresses ?? [])]
      .filter((address) => {
        const normalized = address.address.trim().toLocaleLowerCase();
        return normalized && normalized !== ownEmail && normalized !== senderEmail;
      })
      .map(mailAddressToComposerAddress),
  );

  return {
    cc,
    to: dedupeComposerAddresses(to),
  };
}

function composerAddressFromSender(message: MailMessage) {
  const address = message.senderEmail.trim();
  if (!address) {
    return null;
  }

  return {
    address,
    name:
      message.sender.trim() && message.sender.trim() !== address
        ? message.sender.trim()
        : null,
  };
}

function mailAddressToComposerAddress(address: {
  name?: string | null;
  address: string;
}) {
  return {
    address: address.address.trim(),
    name: address.name?.trim() || null,
  };
}

function dedupeComposerAddresses<T extends { address: string }>(addresses: T[]) {
  const seen = new Set<string>();
  const deduped: T[] = [];
  for (const address of addresses) {
    const normalized = address.address.trim().toLocaleLowerCase();
    if (!normalized || seen.has(normalized)) {
      continue;
    }
    seen.add(normalized);
    deduped.push(address);
  }
  return deduped;
}

function subjectForRoute(
  subject: string,
  route: ComposerRouteKind,
  t: TFunction,
) {
  const trimmed = subject.trim();
  const fallbackSubject = t("mail.messages.noSubject");
  if (route === "forward") {
    return /^\s*(fw|fwd)\s*:/i.test(trimmed)
      ? trimmed
      : `Fwd: ${trimmed || fallbackSubject}`;
  }

  return /^\s*re\s*:/i.test(trimmed)
    ? trimmed
    : `Re: ${trimmed || fallbackSubject}`;
}

function replyReferencesForMessage(message: MailMessage) {
  const references = [...(message.references ?? [])];
  if (message.rfcMessageId) {
    references.push(message.rfcMessageId);
  }

  return dedupeMessageIds(references);
}

function dedupeMessageIds(values: string[]) {
  const seen = new Set<string>();
  const deduped: string[] = [];
  for (const value of values) {
    const trimmed = value.trim();
    const normalized = trimmed.toLocaleLowerCase();
    if (
      !trimmed ||
      !trimmed.startsWith("<") ||
      !trimmed.endsWith(">") ||
      !trimmed.includes("@") ||
      seen.has(normalized)
    ) {
      continue;
    }
    seen.add(normalized);
    deduped.push(trimmed);
  }
  return deduped;
}

function quotedBodyFromMessage(
  message: MailMessage,
  route: ComposerRouteKind,
  localeFormat: ReturnType<typeof useLocaleFormatters>,
  t: TFunction,
) {
  const sender = messageSenderLabel(message, t);
  const receivedAt = localeFormat.dateTime(message.receivedAt);
  const plain = messagePlainText(message);
  const quotedHtml = quotedHtmlFromText(plain);
  const quotedText = quotePlainText(plain);

  if (route === "forward") {
    const toLine = (message.toAddresses ?? [])
      .map(formatComposerAddressForDisplay)
      .join(", ");
    const ccLine = (message.ccAddresses ?? [])
      .map(formatComposerAddressForDisplay)
      .join(", ");
    const headerRows = [
      `<p><strong>${escapeHtml(t("composer.routing.forwardHeader"))}</strong></p>`,
      `<p><strong>${escapeHtml(t("composer.routing.from"))}:</strong> ${escapeHtml(
        sender,
      )}</p>`,
      `<p><strong>${escapeHtml(t("composer.routing.date"))}:</strong> ${escapeHtml(
        receivedAt,
      )}</p>`,
      `<p><strong>${escapeHtml(t("composer.routing.subject"))}:</strong> ${escapeHtml(
        message.subject || t("mail.messages.noSubject"),
      )}</p>`,
      toLine
        ? `<p><strong>${escapeHtml(t("composer.routing.to"))}:</strong> ${escapeHtml(
            toLine,
          )}</p>`
        : "",
      ccLine
        ? `<p><strong>${escapeHtml(t("composer.routing.cc"))}:</strong> ${escapeHtml(
            ccLine,
          )}</p>`
        : "",
    ].join("");
    const textHeader = [
      t("composer.routing.forwardHeader"),
      `${t("composer.routing.from")}: ${sender}`,
      `${t("composer.routing.date")}: ${receivedAt}`,
      `${t("composer.routing.subject")}: ${
        message.subject || t("mail.messages.noSubject")
      }`,
      toLine ? `${t("composer.routing.to")}: ${toLine}` : "",
      ccLine ? `${t("composer.routing.cc")}: ${ccLine}` : "",
    ]
      .filter(Boolean)
      .join("\n");

    return {
      bodyHtml: `<p></p><blockquote>${headerRows}${quotedHtml}</blockquote>`,
      bodyText: `\n\n${textHeader}\n\n${quotedText}`,
    };
  }

  const intro = t("composer.routing.replyIntro", {
    date: receivedAt,
    sender,
  });

  return {
    bodyHtml: `<p></p><blockquote><p>${escapeHtml(intro)}</p>${quotedHtml}</blockquote>`,
    bodyText: `\n\n${intro}\n${quotedText}`,
  };
}

function messagePlainText(message: MailMessage) {
  const bodyText = message.bodyText?.trim();
  if (bodyText) {
    return bodyText;
  }
  if (message.bodyHtml) {
    return htmlToText(message.bodyHtml).trim() || message.preview;
  }
  return message.preview;
}

function htmlToText(html: string) {
  return sanitizeEmailHtml(html).plainText;
}

function quotedHtmlFromText(text: string) {
  return text
    .split(/\n{2,}/)
    .map((paragraph) => paragraph.trim())
    .filter(Boolean)
    .map(
      (paragraph) =>
        `<p>${escapeHtml(paragraph).replace(/\n/g, "<br>")}</p>`,
    )
    .join("");
}

function quotePlainText(text: string) {
  return text
    .split(/\r?\n/)
    .map((line) => `> ${line}`)
    .join("\n");
}

function escapeHtml(value: string) {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function formatComposerAddressForDisplay(address: {
  name?: string | null;
  address: string;
}) {
  const trimmedAddress = address.address.trim();
  const trimmedName = address.name?.trim();
  return trimmedName ? `${trimmedName} <${trimmedAddress}>` : trimmedAddress;
}

function createRoutedDraftId(route: ComposerRouteKind, messageId: string) {
  const suffix =
    globalThis.crypto?.randomUUID?.() ??
    `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  return `draft_${route.replace(/[^a-z]/g, "_")}_${messageId}_${suffix}`.replace(
    /[^A-Za-z0-9_-]/g,
    "_",
  );
}

function messageIdFromActionResponse(response: MailActionResponse) {
  return response.undo?.messageId ?? null;
}

function foldersAfterMailAction(
  folders: MailShellData["folders"],
  messages: MailShellData["messages"],
  response: MailActionResponse,
) {
  const changedMessageId = messageIdFromActionResponse(response);
  const changedMessage = changedMessageId
    ? messages.find(
        (message) =>
          message.accountId === response.accountId &&
          message.id === changedMessageId,
      ) ?? null
    : null;

  return folders.map((folder) => {
    if (response.mutation.kind === "flags-updated" && changedMessage) {
      if (
        folder.accountId !== response.accountId ||
        folder.id !== changedMessage.folderId ||
        changedMessage.unread === !response.mutation.read
      ) {
        return folder;
      }

      return {
        ...folder,
        unreadCount: Math.max(
          0,
          folder.unreadCount + (response.mutation.read ? -1 : 1),
        ),
      };
    }

    if (response.mutation.kind === "moved" && changedMessage) {
      if (
        folder.accountId === response.accountId &&
        folder.id === response.mutation.fromFolderId
      ) {
        return {
          ...folder,
          totalCount: Math.max(0, folder.totalCount - 1),
          unreadCount: Math.max(
            0,
            folder.unreadCount - (changedMessage.unread ? 1 : 0),
          ),
        };
      }

      if (
        folder.accountId === response.accountId &&
        folder.id === response.mutation.toFolderId
      ) {
        return {
          ...folder,
          totalCount: folder.totalCount + 1,
          unreadCount: folder.unreadCount + (changedMessage.unread ? 1 : 0),
        };
      }
    }

    return folder;
  });
}

function accountsWithFolderUnreadCounts(
  accounts: MailShellData["accounts"],
  folders: MailShellData["folders"],
) {
  return accounts.map((account) => ({
    ...account,
    unreadCount: folders
      .filter((folder) => folder.accountId === account.id)
      .reduce((total, folder) => total + folder.unreadCount, 0),
  }));
}

function mailActionRunningMessage(action: MailActionKind, t: TFunction) {
  return t(`mail.reader.mailActions.running.${action.kind}`);
}

function mailActionSuccessMessage(response: MailActionResponse, t: TFunction) {
  switch (response.mutation.kind) {
    case "flags-updated":
      return response.mutation.read
        ? t("mail.reader.mailActions.success.markRead")
        : t("mail.reader.mailActions.success.markUnread");
    case "moved":
      if (response.action.kind === "archive") {
        return t("mail.reader.mailActions.success.archive");
      }
      if (response.action.kind === "delete-to-trash") {
        return t("mail.reader.mailActions.success.deleteToTrash");
      }
      return t("mail.reader.mailActions.success.move");
    case "expunged":
      return t("mail.reader.mailActions.success.expunge", {
        count: response.mutation.removedCount,
      });
  }

  return t("mail.reader.mailActions.success.move");
}

function retryDecisionFromError(
  error: unknown,
): MailActionRetryDecision | null {
  if (
    error &&
    typeof error === "object" &&
    "retry" in error &&
    typeof error.retry === "object" &&
    error.retry &&
    "kind" in error.retry
  ) {
    return error.retry as MailActionRetryDecision;
  }

  return null;
}

function mailActionFailureMessage(error: unknown, t: TFunction) {
  const reason =
    error &&
    typeof error === "object" &&
    "reason" in error &&
    typeof (error as MailActionErrorResponse).reason === "string"
      ? (error as MailActionErrorResponse).reason
      : null;

  return reason || t("mail.reader.mailActions.error");
}

function triggerBrowserDownload(download: AttachmentDownloadResponse) {
  const blob = new Blob([new Uint8Array(download.bytes)], {
    type: download.contentType,
  });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = download.displayFilename;
  link.rel = "noopener";
  document.body.append(link);
  link.click();
  link.remove();
  URL.revokeObjectURL(url);
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
