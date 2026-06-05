import type { MailClientShellProps } from "../mail/shell/MailClientShell";
import {
  allowRemoteContentFromTauri,
  autosaveComposerDraftFromTauri,
  createAccountFromTauri,
  discoverAccountSettingsFromTauri,
  discardComposerDraftFromTauri,
  downloadAttachmentFromTauri,
  exportDiagnosticsBundleFromTauri,
  getAccountAuthStatusFromTauri,
  getDiagnosticsHealthFromTauri,
  getUiPreferencesFromTauri,
  isTauriRuntime,
  listComposerDraftsFromTauri,
  listRecentRecipientsFromTauri,
  loadMailShellDataFromTauri,
  loadMessagePageFromTauri,
  openExternalLinkFromTauri,
  performMailActionFromTauri,
  previewAttachmentFromTauri,
  reauthAccountFromTauri,
  removeAccountFromTauri,
  saveAttachmentFromTauri,
  searchMailFromTauri,
  sendComposerMailFromTauri,
  stageComposerAttachmentFromTauri,
  stageForwardedAttachmentsFromTauri,
  startOAuthAccountReauthFromTauri,
  startOAuthAccountSetupFromTauri,
  subscribeToSyncStatusFromTauri,
  testAccountConnectionFromTauri,
  updateAccountSettingsFromTauri,
  updateUiPreferencesFromTauri,
} from "../mail/shell/mailShellDataSource";

export type KomorebiAppProps = MailClientShellProps & {
  routerBasepath?: string;
};

export function createMailClientProps(
  props: KomorebiAppProps,
): MailClientShellProps {
  const isRuntime = isTauriRuntime();
  const runtimeLoader = isRuntime ? loadMailShellDataFromTauri : undefined;
  const runtimeMessagePageLoader = isRuntime
    ? loadMessagePageFromTauri
    : undefined;
  const runtimeSearch = isRuntime ? searchMailFromTauri : undefined;
  const runtimeAllowRemoteContent = isRuntime
    ? allowRemoteContentFromTauri
    : undefined;
  const runtimeUiPreferences = isRuntime ? getUiPreferencesFromTauri : undefined;
  const runtimeUpdateUiPreferences = isRuntime
    ? updateUiPreferencesFromTauri
    : undefined;
  const runtimeOpenExternalLink = isRuntime
    ? openExternalLinkFromTauri
    : undefined;
  const runtimeMailAction = isRuntime ? performMailActionFromTauri : undefined;
  const runtimeAttachmentPreview = isRuntime
    ? previewAttachmentFromTauri
    : undefined;
  const runtimeAttachmentSave = isRuntime ? saveAttachmentFromTauri : undefined;
  const runtimeAttachmentDownload = isRuntime
    ? downloadAttachmentFromTauri
    : undefined;
  const runtimeDraftSave = isRuntime ? autosaveComposerDraftFromTauri : undefined;
  const runtimeDraftDiscard = isRuntime
    ? discardComposerDraftFromTauri
    : undefined;
  const runtimeAttachmentStage = isRuntime
    ? stageComposerAttachmentFromTauri
    : undefined;
  const runtimeForwardedAttachmentStage = isRuntime
    ? stageForwardedAttachmentsFromTauri
    : undefined;
  const runtimeSend = isRuntime ? sendComposerMailFromTauri : undefined;
  const runtimeRecentRecipients = isRuntime
    ? listRecentRecipientsFromTauri
    : undefined;
  const runtimeComposerDrafts = isRuntime
    ? listComposerDraftsFromTauri
    : undefined;
  const runtimeCreateAccount = isRuntime ? createAccountFromTauri : undefined;
  const runtimeDiscoverAccountSettings = isRuntime
    ? discoverAccountSettingsFromTauri
    : undefined;
  const runtimeReauthAccount = isRuntime ? reauthAccountFromTauri : undefined;
  const runtimeStartOAuthAccountSetup = isRuntime
    ? startOAuthAccountSetupFromTauri
    : undefined;
  const runtimeStartOAuthAccountReauth = isRuntime
    ? startOAuthAccountReauthFromTauri
    : undefined;
  const runtimeRemoveAccount = isRuntime ? removeAccountFromTauri : undefined;
  const runtimeUpdateAccountSettings = isRuntime
    ? updateAccountSettingsFromTauri
    : undefined;
  const runtimeTestAccount = isRuntime ? testAccountConnectionFromTauri : undefined;
  const runtimeGetAccountAuthStatus = isRuntime
    ? getAccountAuthStatusFromTauri
    : undefined;
  const runtimeExportDiagnostics = isRuntime
    ? exportDiagnosticsBundleFromTauri
    : undefined;
  const runtimeDiagnosticsHealth = isRuntime
    ? getDiagnosticsHealthFromTauri
    : undefined;
  const runtimeSyncStatusSubscription = isRuntime
    ? subscribeToSyncStatusFromTauri
    : undefined;

  return {
    ...props,
    allowRemoteContent: props.allowRemoteContent ?? runtimeAllowRemoteContent,
    createAccount: props.createAccount ?? runtimeCreateAccount,
    discoverAccountSettings:
      props.discoverAccountSettings ?? runtimeDiscoverAccountSettings,
    discardComposerDraft: props.discardComposerDraft ?? runtimeDraftDiscard,
    downloadAttachment: props.downloadAttachment ?? runtimeAttachmentDownload,
    exportDiagnosticsBundle:
      props.exportDiagnosticsBundle ?? runtimeExportDiagnostics,
    getAccountAuthStatus:
      props.getAccountAuthStatus ?? runtimeGetAccountAuthStatus,
    loadComposerDrafts: props.loadComposerDrafts ?? runtimeComposerDrafts,
    loadDiagnosticsHealth:
      props.loadDiagnosticsHealth ?? runtimeDiagnosticsHealth,
    loadMailShellData: props.loadMailShellData ?? runtimeLoader,
    loadMessagePage: props.loadMessagePage ?? runtimeMessagePageLoader,
    loadRecentRecipients: props.loadRecentRecipients ?? runtimeRecentRecipients,
    loadUiPreferences: props.loadUiPreferences ?? runtimeUiPreferences,
    openExternalLink: props.openExternalLink ?? runtimeOpenExternalLink,
    performMailAction: props.performMailAction ?? runtimeMailAction,
    previewAttachment: props.previewAttachment ?? runtimeAttachmentPreview,
    reauthAccount: props.reauthAccount ?? runtimeReauthAccount,
    removeAccount: props.removeAccount ?? runtimeRemoveAccount,
    saveAttachment: props.saveAttachment ?? runtimeAttachmentSave,
    saveComposerDraft: props.saveComposerDraft ?? runtimeDraftSave,
    searchMail: props.searchMail ?? runtimeSearch,
    sendComposerMail: props.sendComposerMail ?? runtimeSend,
    stageComposerAttachment:
      props.stageComposerAttachment ?? runtimeAttachmentStage,
    stageForwardedAttachments:
      props.stageForwardedAttachments ?? runtimeForwardedAttachmentStage,
    startOAuthAccountReauth:
      props.startOAuthAccountReauth ?? runtimeStartOAuthAccountReauth,
    startOAuthAccountSetup:
      props.startOAuthAccountSetup ?? runtimeStartOAuthAccountSetup,
    subscribeToSyncStatus:
      props.subscribeToSyncStatus ?? runtimeSyncStatusSubscription,
    testAccountConnection: props.testAccountConnection ?? runtimeTestAccount,
    updateAccountSettings:
      props.updateAccountSettings ?? runtimeUpdateAccountSettings,
    updateUiPreferences: props.updateUiPreferences ?? runtimeUpdateUiPreferences,
  };
}
