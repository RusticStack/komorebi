import { useMemo, useState, type ReactNode } from "react";
import { SafeEmailBody } from "../rendering/SafeEmailBody";
import { AccountSignatureSettings } from "../../settings/AccountSignatureSettings";
import {
  createMailShellState,
  emptyMailShellData,
  foldersForScope,
  messagesForScope,
  readyMailShellLoadState,
  selectedAccount,
  selectedFolder,
  selectedMessage,
  type MailShellData,
  type MailShellLoadState,
  type MailShellState,
} from "./mailShellState";

export type MailClientShellProps = {
  initialData?: MailShellData;
  initialLoadState?: MailShellLoadState;
};

type Drawer = "composer" | "settings" | null;

export function MailClientShell({
  initialData = emptyMailShellData,
  initialLoadState = readyMailShellLoadState,
}: MailClientShellProps) {
  const [state, setState] = useState<MailShellState>(() =>
    createMailShellState(initialData, initialLoadState),
  );
  const [drawer, setDrawer] = useState<Drawer>(null);
  const visibleFolders = useMemo(() => foldersForScope(state), [state]);
  const visibleMessages = useMemo(() => messagesForScope(state), [state]);
  const activeMessage = selectedMessage(state);
  const activeAccount = selectedAccount(state);
  const activeFolder = selectedFolder(state);
  const hasAccounts = state.data.accounts.length > 0;
  const canCompose = hasAccounts && state.loadState.accounts === "ready";
  const listTitle = state.unifiedInbox
    ? "Unified Inbox"
    : activeFolder?.name ?? activeAccount?.label ?? "Inbox";

  function patchState(update: Partial<MailShellState>) {
    setState((current) => ({
      ...current,
      ...update,
    }));
  }

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
            <button
              className="icon-command"
              type="button"
              aria-label="Add account"
              disabled
            >
              +
            </button>
          </div>

          <label className="unified-toggle">
            <input
              type="checkbox"
              checked={state.unifiedInbox}
              disabled={!hasAccounts}
              onChange={(event) =>
                patchState({
                  unifiedInbox: event.target.checked,
                  selectedAccountId: event.target.checked
                    ? state.data.accounts[0]?.id ?? null
                    : state.selectedAccountId ?? state.data.accounts[0]?.id ?? null,
                  selectedFolderId: null,
                  selectedMessageId: null,
                })
              }
            />
            <span>Unified inbox</span>
          </label>

          <StatusList
            status={state.loadState.accounts}
            loadingLabel="Loading accounts"
            errorLabel="Account list unavailable"
            empty={state.data.accounts.length === 0}
            emptyLabel="No accounts configured."
          >
            {state.data.accounts.map((account) => (
              <button
                className={
                  !state.unifiedInbox && account.id === state.selectedAccountId
                    ? "account-button is-active"
                    : "account-button"
                }
                type="button"
                key={account.id}
                onClick={() =>
                  patchState({
                    unifiedInbox: false,
                    selectedAccountId: account.id,
                    selectedFolderId: null,
                    selectedMessageId: null,
                  })
                }
              >
                <span>{account.label}</span>
                <small>{account.email}</small>
                <strong>{account.status}</strong>
              </button>
            ))}
          </StatusList>
        </section>

        <nav className="sidebar-section" aria-label="Folders">
          <h2>Folders</h2>
          <StatusList
            status={state.loadState.folders}
            loadingLabel="Loading folders"
            errorLabel="Folder list unavailable"
            empty={visibleFolders.length === 0}
            emptyLabel="Folders appear after an account is added."
          >
            <button
              className={!state.selectedFolderId ? "folder is-active" : "folder"}
              type="button"
              onClick={() =>
                patchState({
                  selectedFolderId: null,
                  selectedMessageId: null,
                })
              }
            >
              <span>{state.unifiedInbox ? "Unified Inbox" : "All mail"}</span>
              <span>{unreadCountForScope(state)}</span>
            </button>
            {visibleFolders.map((folder) => (
              <button
                className={
                  folder.id === state.selectedFolderId
                    ? "folder is-active"
                    : "folder"
                }
                type="button"
                key={folder.id}
                onClick={() =>
                  patchState({
                    selectedFolderId: folder.id,
                    selectedMessageId: null,
                  })
                }
              >
                <span>{folder.name}</span>
                <span>{folder.unreadCount}</span>
              </button>
            ))}
          </StatusList>
        </nav>

        <section className="sidebar-section sidebar-section--footer" aria-label="App">
          <button
            className="settings-button"
            type="button"
            onClick={() => setDrawer("settings")}
          >
            Settings
          </button>
        </section>
      </aside>

      <section className="message-list" aria-label="Messages">
        <header className="list-toolbar">
          <div>
            <p className="eyebrow">
              {state.unifiedInbox ? "All accounts" : activeAccount?.label ?? "Local"}
            </p>
            <h2>{listTitle}</h2>
          </div>
          <button
            type="button"
            disabled={!canCompose}
            onClick={() => setDrawer("composer")}
          >
            Compose
          </button>
        </header>

        <label className="search">
          <span>Search mail</span>
          <input
            type="search"
            placeholder="Sender, subject, or body"
            disabled={!hasAccounts || state.loadState.messages !== "ready"}
            value={state.searchQuery}
            onChange={(event) =>
              patchState({
                searchQuery: event.target.value,
                selectedMessageId: null,
              })
            }
          />
        </label>

        <div className="messages">
          <MessageListState
            state={state}
            visibleMessages={visibleMessages}
            onSelectMessage={(messageId) =>
              patchState({
                selectedMessageId: messageId,
              })
            }
          />
        </div>
      </section>

      <section className="reader" aria-label="Selected message">
        <ReaderPane
          state={state}
          message={activeMessage}
          hasVisibleMessages={visibleMessages.length > 0}
        />
      </section>

      {drawer === "settings" ? (
        <div className="settings-drawer">
          <AccountSignatureSettings
            accounts={state.data.accounts.map((account) => ({
              id: account.id,
              label: account.label,
              email: account.email,
              signatureHtml: account.signatureHtml,
            }))}
            initialAccountId={state.selectedAccountId}
            onSaveSignature={(accountId, signatureHtml) => {
              setState((current) => ({
                ...current,
                data: {
                  ...current.data,
                  accounts: current.data.accounts.map((account) =>
                    account.id === accountId
                      ? { ...account, signatureHtml }
                      : account,
                  ),
                },
              }));
            }}
            onClose={() => setDrawer(null)}
          />
        </div>
      ) : null}

      {drawer === "composer" ? (
        <ComposerDrawer
          accountLabel={activeAccount?.label ?? state.data.accounts[0]?.label ?? ""}
          onClose={() => setDrawer(null)}
        />
      ) : null}
    </main>
  );
}

function StatusList({
  status,
  loadingLabel,
  errorLabel,
  empty,
  emptyLabel,
  children,
}: {
  status: MailShellLoadState[keyof MailShellLoadState];
  loadingLabel: string;
  errorLabel: string;
  empty: boolean;
  emptyLabel: string;
  children: ReactNode;
}) {
  if (status === "loading") {
    return <p className="empty-note status-note">{loadingLabel}</p>;
  }

  if (status === "error") {
    return (
      <div className="status-note status-note--error" role="status">
        <p>{errorLabel}</p>
        <button type="button">Retry</button>
      </div>
    );
  }

  if (empty) {
    return <p className="empty-note">{emptyLabel}</p>;
  }

  return children;
}

function MessageListState({
  state,
  visibleMessages,
  onSelectMessage,
}: {
  state: MailShellState;
  visibleMessages: ReturnType<typeof messagesForScope>;
  onSelectMessage: (messageId: string) => void;
}) {
  if (state.loadState.messages === "loading") {
    return (
      <section className="empty-panel" aria-label="Loading messages">
        <h3>Loading mail</h3>
        <p>Reading the local cache.</p>
      </section>
    );
  }

  if (state.loadState.messages === "error") {
    return (
      <section className="empty-panel error-panel" aria-label="Message error">
        <h3>Mail unavailable</h3>
        <p>The local message cache could not be opened.</p>
        <button type="button">Retry</button>
      </section>
    );
  }

  if (state.data.accounts.length === 0) {
    return (
      <section className="empty-panel" aria-label="Empty inbox">
        <h3>No mail loaded</h3>
        <p>Add an account to sync folders and messages into the local cache.</p>
      </section>
    );
  }

  if (visibleMessages.length === 0 && state.searchQuery.trim()) {
    return (
      <section className="empty-panel" aria-label="No search results">
        <h3>No matches</h3>
        <p>Search found no local messages in the selected scope.</p>
      </section>
    );
  }

  if (visibleMessages.length === 0) {
    return (
      <section className="empty-panel" aria-label="Empty folder">
        <h3>No messages</h3>
        <p>This folder has no local mail yet.</p>
      </section>
    );
  }

  return visibleMessages.map((message) => (
    <button
      className={message.unread ? "message is-unread" : "message"}
      key={message.id}
      type="button"
      aria-current={message.id === state.selectedMessageId ? "true" : undefined}
      onClick={() => onSelectMessage(message.id)}
    >
      <div className="message-row">
        <strong>{message.sender}</strong>
        <time>{message.receivedAt}</time>
      </div>
      <h3>{message.subject || "(no subject)"}</h3>
      <p>{message.preview}</p>
    </button>
  ));
}

function ReaderPane({
  state,
  message,
  hasVisibleMessages,
}: {
  state: MailShellState;
  message: ReturnType<typeof selectedMessage>;
  hasVisibleMessages: boolean;
}) {
  if (state.loadState.messages === "loading") {
    return (
      <>
        <section className="reader-empty" aria-label="Reader loading">
          <p className="eyebrow">Reader</p>
          <h2>Loading message view</h2>
          <p>The local cache is being prepared.</p>
        </section>
        <ReaderStatus text="Loading local cache" />
      </>
    );
  }

  if (state.loadState.messages === "error") {
    return (
      <>
        <section className="reader-empty" aria-label="Reader error">
          <p className="eyebrow">Reader</p>
          <h2>Message view unavailable</h2>
          <p>Resolve the cache error before opening messages.</p>
        </section>
        <ReaderStatus text="Error state" />
      </>
    );
  }

  if (!message) {
    return (
      <>
        <section className="reader-empty" aria-label="No selected message">
          <p className="eyebrow">Reader</p>
          <h2>No message selected</h2>
          <p>
            {hasVisibleMessages
              ? "Select a message from the list to read it."
              : "Messages appear here after account setup and sync complete."}
          </p>
        </section>
        <ReaderStatus
          text={
            state.data.accounts.length === 0
              ? "Account setup required"
              : "Remote content blocked by default"
          }
        />
      </>
    );
  }

  return (
    <>
      <header className="reader-header">
        <p className="eyebrow">Selected message</p>
        <h2>{message.subject || "(no subject)"}</h2>
        <div className="sender-line">
          <span>{message.sender}</span>
          <time>{message.receivedAt}</time>
        </div>
      </header>

      <div className="reader-actions" aria-label="Message actions">
        <button type="button">Reply</button>
        <button type="button">Archive</button>
        <button type="button">{message.unread ? "Mark read" : "Mark unread"}</button>
      </div>

      <article className="reader-body">
        <SafeEmailBody bodyHtml={message.bodyHtml} bodyText={message.bodyText} />
      </article>

      <ReaderStatus text="Remote content blocked by default" />
    </>
  );
}

function ReaderStatus({ text }: { text: string }) {
  return (
    <footer className="reader-status">
      <span>{text}</span>
      <span>Local cache</span>
    </footer>
  );
}

function ComposerDrawer({
  accountLabel,
  onClose,
}: {
  accountLabel: string;
  onClose: () => void;
}) {
  return (
    <div className="composer-drawer">
      <section className="composer-panel" aria-label="Composer">
        <header className="settings-panel__header">
          <div>
            <p className="eyebrow">{accountLabel || "Composer"}</p>
            <h2>New message</h2>
          </div>
          <button type="button" onClick={onClose}>
            Close
          </button>
        </header>

        <form className="composer-form">
          <label>
            <span>To</span>
            <input type="text" placeholder="recipient@example.com" />
          </label>
          <label>
            <span>Subject</span>
            <input type="text" />
          </label>
          <label>
            <span>Message</span>
            <textarea rows={10} />
          </label>

          <div className="settings-panel__actions">
            <button type="button">Save draft</button>
            <button type="button">Send</button>
          </div>
        </form>
      </section>
    </div>
  );
}

function unreadCountForScope(state: MailShellState) {
  return messagesForScope({
    ...state,
    selectedFolderId: null,
    searchQuery: "",
  }).filter((message) => message.unread).length;
}
