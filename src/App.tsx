import "./App.css";
import {
  MailClientShell,
  type MailClientShellProps,
} from "./mail/shell/MailClientShell";
import {
  autosaveComposerDraftFromTauri,
  createAccountFromTauri,
  discardComposerDraftFromTauri,
  getAccountAuthStatusFromTauri,
  isTauriRuntime,
  listRecentRecipientsFromTauri,
  loadMailShellDataFromTauri,
  reauthAccountFromTauri,
  removeAccountFromTauri,
  searchMailFromTauri,
  sendComposerMailFromTauri,
  stageComposerAttachmentFromTauri,
  testAccountConnectionFromTauri,
} from "./mail/shell/mailShellDataSource";

function App(props: MailClientShellProps) {
  const isRuntime = isTauriRuntime();
  const runtimeLoader = isRuntime ? loadMailShellDataFromTauri : undefined;
  const runtimeSearch = isRuntime ? searchMailFromTauri : undefined;
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
  const runtimeCreateAccount = isRuntime ? createAccountFromTauri : undefined;
  const runtimeReauthAccount = isRuntime ? reauthAccountFromTauri : undefined;
  const runtimeRemoveAccount = isRuntime ? removeAccountFromTauri : undefined;
  const runtimeTestAccount = isRuntime
    ? testAccountConnectionFromTauri
    : undefined;
  const runtimeGetAccountAuthStatus = isRuntime
    ? getAccountAuthStatusFromTauri
    : undefined;

  return (
    <MailClientShell
      {...props}
      loadMailShellData={props.loadMailShellData ?? runtimeLoader}
      searchMail={props.searchMail ?? runtimeSearch}
      saveComposerDraft={props.saveComposerDraft ?? runtimeDraftSave}
      discardComposerDraft={props.discardComposerDraft ?? runtimeDraftDiscard}
      stageComposerAttachment={
        props.stageComposerAttachment ?? runtimeAttachmentStage
      }
      sendComposerMail={props.sendComposerMail ?? runtimeSend}
      loadRecentRecipients={
        props.loadRecentRecipients ?? runtimeRecentRecipients
      }
      createAccount={props.createAccount ?? runtimeCreateAccount}
      reauthAccount={props.reauthAccount ?? runtimeReauthAccount}
      removeAccount={props.removeAccount ?? runtimeRemoveAccount}
      testAccountConnection={props.testAccountConnection ?? runtimeTestAccount}
      getAccountAuthStatus={
        props.getAccountAuthStatus ?? runtimeGetAccountAuthStatus
      }
    />
  );
}

export default App;
