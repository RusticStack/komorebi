import "./App.css";
import {
  MailClientShell,
  type MailClientShellProps,
} from "./mail/shell/MailClientShell";

function App(props: MailClientShellProps) {
  return <MailClientShell {...props} />;
}

export default App;
