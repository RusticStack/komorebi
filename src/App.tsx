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
  getAccountAuthStatusFromTauri,
  isTauriRuntime,
  listComposerDraftsFromTauri,
  listRecentRecipientsFromTauri,
  loadMailShellDataFromTauri,
  openExternalLinkFromTauri,
  previewAttachmentFromTauri,
  reauthAccountFromTauri,
  removeAccountFromTauri,
  saveAttachmentFromTauri,
  searchMailFromTauri,
  sendComposerMailFromTauri,
  stageComposerAttachmentFromTauri,
  startOAuthAccountReauthFromTauri,
  startOAuthAccountSetupFromTauri,
  subscribeToSyncStatusFromTauri,
  testAccountConnectionFromTauri,
  updateAccountSettingsFromTauri,
} from "./mail/shell/mailShellDataSource";

function App(props: MailClientShellProps) {
  const isRuntime = isTauriRuntime();
  const runtimeLoader = isRuntime ? loadMailShellDataFromTauri : undefined;
  const runtimeSearch = isRuntime ? searchMailFromTauri : undefined;
  const runtimeAllowRemoteContent = isRuntime
    ? allowRemoteContentFromTauri
    : undefined;
  const runtimeOpenExternalLink = isRuntime
    ? openExternalLinkFromTauri
    : undefined;
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
  const runtimeSyncStatusSubscription = isRuntime
    ? subscribeToSyncStatusFromTauri
    : undefined;

  return (
    <MailClientShell
      {...props}
      loadMailShellData={props.loadMailShellData ?? runtimeLoader}
      searchMail={props.searchMail ?? runtimeSearch}
      allowRemoteContent={
        props.allowRemoteContent ?? runtimeAllowRemoteContent
      }
      openExternalLink={props.openExternalLink ?? runtimeOpenExternalLink}
      previewAttachment={props.previewAttachment ?? runtimeAttachmentPreview}
      saveAttachment={props.saveAttachment ?? runtimeAttachmentSave}
      downloadAttachment={props.downloadAttachment ?? runtimeAttachmentDownload}
      saveComposerDraft={props.saveComposerDraft ?? runtimeDraftSave}
      discardComposerDraft={props.discardComposerDraft ?? runtimeDraftDiscard}
      stageComposerAttachment={
        props.stageComposerAttachment ?? runtimeAttachmentStage
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
      subscribeToSyncStatus={
        props.subscribeToSyncStatus ?? runtimeSyncStatusSubscription
      }
    />
  );
}

export default App;
