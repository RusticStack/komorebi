import "./App.css";

const folders = [
  { name: "Unified inbox", count: 18, active: true },
  { name: "Inbox", count: 8, active: false },
  { name: "Sent", count: 0, active: false },
  { name: "Drafts", count: 2, active: false },
  { name: "Archive", count: 0, active: false },
];

const messages = [
  {
    sender: "Rui Costa",
    subject: "Client kickoff agenda",
    preview: "Updated timing, owners, and attachments are ready for review.",
    time: "09:42",
    unread: true,
  },
  {
    sender: "Olivia Martin",
    subject: "Design notes for the Portugal launch",
    preview: "The muted sage accent works well with the denser inbox view.",
    time: "Yesterday",
    unread: true,
  },
  {
    sender: "Security digest",
    subject: "Mailbox rule audit",
    preview: "No forwarding rules changed across connected accounts.",
    time: "Tue",
    unread: false,
  },
  {
    sender: "Komorebi",
    subject: "Local cache status",
    preview: "SQLite is ready for migrations once account sync is implemented.",
    time: "Mon",
    unread: false,
  },
];

function App() {
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
          <h2>Accounts</h2>
          <button className="account-button" type="button">
            <span>personal@example.com</span>
            <strong>Online</strong>
          </button>
        </section>

        <nav className="sidebar-section" aria-label="Folders">
          <h2>Folders</h2>
          {folders.map((folder) => (
            <button
              className={folder.active ? "folder is-active" : "folder"}
              type="button"
              key={folder.name}
            >
              <span>{folder.name}</span>
              <span>{folder.count}</span>
            </button>
          ))}
        </nav>
      </aside>

      <section className="message-list" aria-label="Messages">
        <header className="list-toolbar">
          <div>
            <p className="eyebrow">Unified</p>
            <h2>Inbox</h2>
          </div>
          <button type="button">Compose</button>
        </header>

        <label className="search">
          <span>Search mail</span>
          <input type="search" placeholder="Sender, subject, or body" />
        </label>

        <div className="messages">
          {messages.map((message, index) => (
            <article
              className={message.unread ? "message is-unread" : "message"}
              key={message.subject}
              aria-current={index === 0 ? "true" : undefined}
            >
              <div className="message-row">
                <strong>{message.sender}</strong>
                <time>{message.time}</time>
              </div>
              <h3>{message.subject}</h3>
              <p>{message.preview}</p>
            </article>
          ))}
        </div>
      </section>

      <section className="reader" aria-label="Selected message">
        <header className="reader-header">
          <p className="eyebrow">personal@example.com</p>
          <h2>Client kickoff agenda</h2>
          <div className="sender-line">
            <span>Rui Costa</span>
            <time>Today, 09:42</time>
          </div>
        </header>

        <div className="reader-actions" aria-label="Message actions">
          <button type="button">Reply</button>
          <button type="button">Archive</button>
          <button type="button">Mark read</button>
        </div>

        <article className="reader-body">
          <p>Hello,</p>
          <p>
            The kickoff outline is ready for review. The first implementation
            pass should focus on account setup, safe local storage, and the
            SQLite-backed message cache before connecting live mailboxes.
          </p>
          <p>
            The attachment queue is empty, and remote content remains blocked.
          </p>
        </article>

        <footer className="reader-status">
          <span>Local cache not connected</span>
          <span>Remote images blocked</span>
        </footer>
      </section>
    </main>
  );
}

export default App;
