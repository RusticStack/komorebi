import "./App.css";
import {
  MailClientShell,
  type MailClientShellProps,
} from "./mail/shell/MailClientShell";
import {
  autosaveComposerDraftFromTauri,
  discardComposerDraftFromTauri,
  isTauriRuntime,
  listRecentRecipientsFromTauri,
  loadMailShellDataFromTauri,
  searchMailFromTauri,
  sendComposerMailFromTauri,
  stageComposerAttachmentFromTauri,
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
    />
  );
}

export default App;
