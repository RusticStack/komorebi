import "./App.css";
import {
  MailClientShell,
  type MailClientShellProps,
} from "./mail/shell/MailClientShell";
import {
  isTauriRuntime,
  loadMailShellDataFromTauri,
  searchMailFromTauri,
} from "./mail/shell/mailShellDataSource";

function App(props: MailClientShellProps) {
  const runtimeLoader = isTauriRuntime() ? loadMailShellDataFromTauri : undefined;
  const runtimeSearch = isTauriRuntime() ? searchMailFromTauri : undefined;

  return (
    <MailClientShell
      {...props}
      loadMailShellData={props.loadMailShellData ?? runtimeLoader}
      searchMail={props.searchMail ?? runtimeSearch}
    />
  );
}

export default App;
