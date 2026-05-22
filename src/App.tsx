import "./App.css";

type AccountSummary = {
  id: string;
  label: string;
  status: "connected" | "reconnect-needed" | "offline";
};

type FolderSummary = {
  id: string;
  name: string;
  unreadCount: number;
};

type MessageSummary = {
  id: string;
  sender: string;
  subject: string;
  preview: string;
  receivedAt: string;
  unread: boolean;
};

type MailShellState = {
  accounts: AccountSummary[];
  folders: FolderSummary[];
  messages: MessageSummary[];
  selectedMessageId: string | null;
};

const initialMailState: MailShellState = {
  accounts: [],
  folders: [],
  messages: [],
  selectedMessageId: null,
};

function App() {
  const hasAccounts = initialMailState.accounts.length > 0;
  const hasMessages = initialMailState.messages.length > 0;
  const selectedMessage = initialMailState.messages.find(
    (message) => message.id === initialMailState.selectedMessageId,
  );

  return (
    <main className="app-shell">
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
            <button className="icon-command" type="button" aria-label="Add account">
              +
            </button>
          </div>

          {hasAccounts ? (
            initialMailState.accounts.map((account) => (
              <button className="account-button" type="button" key={account.id}>
                <span>{account.label}</span>
                <strong>{account.status}</strong>
              </button>
            ))
          ) : (
            <p className="empty-note">No accounts configured.</p>
          )}
        </section>

        <nav className="sidebar-section" aria-label="Folders">
          <h2>Folders</h2>
          {initialMailState.folders.length > 0 ? (
            initialMailState.folders.map((folder, index) => (
              <button
                className={index === 0 ? "folder is-active" : "folder"}
                type="button"
                key={folder.id}
              >
                <span>{folder.name}</span>
                <span>{folder.unreadCount}</span>
              </button>
            ))
          ) : (
            <p className="empty-note">Folders appear after an account is added.</p>
          )}
        </nav>
      </aside>

      <section className="message-list" aria-label="Messages">
        <header className="list-toolbar">
          <div>
            <p className="eyebrow">Local</p>
            <h2>Inbox</h2>
          </div>
          <button type="button" disabled={!hasAccounts}>
            Compose
          </button>
        </header>

        <label className="search">
          <span>Search mail</span>
          <input
            type="search"
            placeholder="Sender, subject, or body"
            disabled={!hasAccounts}
          />
        </label>

        <div className="messages">
          {hasMessages ? (
            initialMailState.messages.map((message) => (
              <article
                className={message.unread ? "message is-unread" : "message"}
                key={message.id}
                aria-current={
                  message.id === initialMailState.selectedMessageId
                    ? "true"
                    : undefined
                }
              >
                <div className="message-row">
                  <strong>{message.sender}</strong>
                  <time>{message.receivedAt}</time>
                </div>
                <h3>{message.subject}</h3>
                <p>{message.preview}</p>
              </article>
            ))
          ) : (
            <section className="empty-panel" aria-label="Empty inbox">
              <h3>No mail loaded</h3>
              <p>Add an account to sync folders and messages into the local cache.</p>
            </section>
          )}
        </div>
      </section>

      <section className="reader" aria-label="Selected message">
        {selectedMessage ? (
          <>
            <header className="reader-header">
              <p className="eyebrow">Selected message</p>
              <h2>{selectedMessage.subject}</h2>
              <div className="sender-line">
                <span>{selectedMessage.sender}</span>
                <time>{selectedMessage.receivedAt}</time>
              </div>
            </header>

            <div className="reader-actions" aria-label="Message actions">
              <button type="button">Reply</button>
              <button type="button">Archive</button>
              <button type="button">Mark read</button>
            </div>

            <article className="reader-body">
              <p>{selectedMessage.preview}</p>
            </article>
          </>
        ) : (
          <section className="reader-empty" aria-label="No selected message">
            <p className="eyebrow">Reader</p>
            <h2>No message selected</h2>
            <p>Messages appear here after account setup and sync complete.</p>
          </section>
        )}

        <footer className="reader-status">
          <span>Account setup required</span>
          <span>Remote content blocked by default</span>
        </footer>
      </section>
    </main>
  );
}

export default App;
