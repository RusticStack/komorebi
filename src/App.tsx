import "./App.css";
import {
  MailClientShell,
  type MailClientShellProps,
} from "./mail/shell/MailClientShell";
import {
  allowRemoteContentFromTauri,
  autosaveComposerDraftFromTauri,
  createAccountFromTauri,
  discardComposerDraftFromTauri,
  downloadAttachmentFromTauri,
  exportDiagnosticsBundleFromTauri,
  getAccountAuthStatusFromTauri,
  getUiPreferencesFromTauri,
  isTauriRuntime,
  listComposerDraftsFromTauri,
  listRecentRecipientsFromTauri,
  loadMessagePageFromTauri,
  loadMailShellDataFromTauri,
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
} from "./mail/shell/mailShellDataSource";

function App(props: MailClientShellProps) {
  const isRuntime = isTauriRuntime();
  const runtimeLoader = isRuntime ? loadMailShellDataFromTauri : undefined;
  const runtimeMessagePageLoader = isRuntime ? loadMessagePageFromTauri : undefined;
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
  const runtimeComposerDrafts = isRuntime ? listComposerDraftsFromTauri : undefined;
  const runtimeCreateAccount = isRuntime ? createAccountFromTauri : undefined;
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
  const runtimeTestAccount = isRuntime
    ? testAccountConnectionFromTauri
    : undefined;
  const runtimeGetAccountAuthStatus = isRuntime
    ? getAccountAuthStatusFromTauri
    : undefined;
  const runtimeExportDiagnostics = isRuntime
    ? exportDiagnosticsBundleFromTauri
    : undefined;
  const runtimeSyncStatusSubscription = isRuntime
    ? subscribeToSyncStatusFromTauri
    : undefined;

  return (
    <MailClientShell
      {...props}
      loadMailShellData={props.loadMailShellData ?? runtimeLoader}
      loadMessagePage={props.loadMessagePage ?? runtimeMessagePageLoader}
      searchMail={props.searchMail ?? runtimeSearch}
      allowRemoteContent={
        props.allowRemoteContent ?? runtimeAllowRemoteContent
      }
      loadUiPreferences={props.loadUiPreferences ?? runtimeUiPreferences}
      updateUiPreferences={
        props.updateUiPreferences ?? runtimeUpdateUiPreferences
      }
      openExternalLink={props.openExternalLink ?? runtimeOpenExternalLink}
      performMailAction={props.performMailAction ?? runtimeMailAction}
      previewAttachment={props.previewAttachment ?? runtimeAttachmentPreview}
      saveAttachment={props.saveAttachment ?? runtimeAttachmentSave}
      downloadAttachment={props.downloadAttachment ?? runtimeAttachmentDownload}
      saveComposerDraft={props.saveComposerDraft ?? runtimeDraftSave}
      discardComposerDraft={props.discardComposerDraft ?? runtimeDraftDiscard}
      stageComposerAttachment={
        props.stageComposerAttachment ?? runtimeAttachmentStage
      }
      stageForwardedAttachments={
        props.stageForwardedAttachments ?? runtimeForwardedAttachmentStage
      }
      sendComposerMail={props.sendComposerMail ?? runtimeSend}
      loadRecentRecipients={
        props.loadRecentRecipients ?? runtimeRecentRecipients
      }
      loadComposerDrafts={props.loadComposerDrafts ?? runtimeComposerDrafts}
      createAccount={props.createAccount ?? runtimeCreateAccount}
      reauthAccount={props.reauthAccount ?? runtimeReauthAccount}
      startOAuthAccountSetup={
        props.startOAuthAccountSetup ?? runtimeStartOAuthAccountSetup
      }
      startOAuthAccountReauth={
        props.startOAuthAccountReauth ?? runtimeStartOAuthAccountReauth
      }
      removeAccount={props.removeAccount ?? runtimeRemoveAccount}
      updateAccountSettings={
        props.updateAccountSettings ?? runtimeUpdateAccountSettings
      }
      testAccountConnection={props.testAccountConnection ?? runtimeTestAccount}
      getAccountAuthStatus={
        props.getAccountAuthStatus ?? runtimeGetAccountAuthStatus
      }
      exportDiagnosticsBundle={
        props.exportDiagnosticsBundle ?? runtimeExportDiagnostics
      }
      subscribeToSyncStatus={
        props.subscribeToSyncStatus ?? runtimeSyncStatusSubscription
      }
    />
  );
}

export default App;
