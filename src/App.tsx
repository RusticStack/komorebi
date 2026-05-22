import "./App.css";
import {
  MailClientShell,
  type MailClientShellProps,
} from "./mail/shell/MailClientShell";
import {
  isTauriRuntime,
  loadMailShellDataFromTauri,
} from "./mail/shell/mailShellDataSource";

function App(props: MailClientShellProps) {
  const runtimeLoader = isTauriRuntime() ? loadMailShellDataFromTauri : undefined;

  return (
    <MailClientShell
      {...props}
      loadMailShellData={props.loadMailShellData ?? runtimeLoader}
    />
  );
}

export default App;
