import type { KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Settings } from "lucide-react";
import { useLocaleFormatters } from "../../i18n";
import {
  foldersForScope,
  type MailShellState,
} from "./mailShellState";
import { StatusList } from "./StatusList";
import {
  accountNeedsAttention,
  accountStatusLabel,
  lastSyncLabel,
} from "./accountStatusDisplay";

export function MailSidebar({
  state,
  visibleFolders,
  onAccountButtonRef,
  onAccountKeyDown,
  onFolderButtonRef,
  onFolderKeyDown,
  onOpenAccountStatus,
  onOpenSettings,
  onRetry,
  onSelectAccount,
  onSelectFolder,
  onSelectUnified,
}: {
  state: MailShellState;
  visibleFolders: MailShellState["data"]["folders"];
  onAccountButtonRef: (
    index: number,
    element: HTMLButtonElement | null,
  ) => void;
  onAccountKeyDown: (
    event: KeyboardEvent<HTMLButtonElement>,
    index: number,
  ) => void;
  onFolderButtonRef: (
    index: number,
    element: HTMLButtonElement | null,
  ) => void;
  onFolderKeyDown: (
    event: KeyboardEvent<HTMLButtonElement>,
    index: number,
  ) => void;
  onOpenAccountStatus: (
    accountId: string,
    restoreFocus: HTMLElement | null,
  ) => void;
  onOpenSettings: (restoreFocus: HTMLElement | null) => void;
  onRetry?: () => void;
  onSelectAccount: (accountId: string) => void;
  onSelectFolder: (folderId: string | null) => void;
  onSelectUnified: (unified: boolean) => void;
}) {
  const { t } = useTranslation();
  const localeFormat = useLocaleFormatters();
  const hasAccounts = state.data.accounts.length > 0;

  return (
    <aside className="sidebar" aria-label={t("app.sections.mailboxes")}>
      <div className="brand">
        <span className="brand-mark" aria-hidden="true">
          K
        </span>
        <div>
          <h1>Komorebi</h1>
          <p>{t("app.brandSubtitle")}</p>
        </div>
      </div>

      <section className="sidebar-section" aria-label={t("app.sections.accounts")}>
        <div className="section-heading">
          <h2>{t("app.sections.accounts")}</h2>
          <button
            className="icon-command"
            type="button"
            aria-label={t("mail.account.add")}
            onClick={(event) => onOpenSettings(event.currentTarget)}
          >
            <Plus aria-hidden="true" size={16} />
          </button>
        </div>

        <label className="unified-toggle">
          <input
            type="checkbox"
            checked={state.unifiedInbox}
            disabled={!hasAccounts}
            onChange={(event) => onSelectUnified(event.target.checked)}
          />
          <span>{t("mail.unifiedToggle")}</span>
        </label>

        <StatusList
          status={state.loadState.accounts}
          loadingLabel={t("mail.states.accountsLoading")}
          errorLabel={t("mail.states.accountsError")}
          empty={state.data.accounts.length === 0}
          emptyLabel={t("mail.states.noAccounts")}
          onRetry={onRetry}
        >
          {state.data.accounts.map((account, index) => (
            <div className="account-row" key={account.id}>
              <button
                className={
                  !state.unifiedInbox && account.id === state.selectedAccountId
                    ? "account-button is-active"
                    : "account-button"
                }
                type="button"
                aria-label={t("mail.account.aria", {
                  email: account.email,
                  label: account.label,
                  status: accountStatusLabel(account.status, t),
                  unread: localeFormat.count(account.unreadCount),
                })}
                ref={(element) => onAccountButtonRef(index, element)}
                onKeyDown={(event) => onAccountKeyDown(event, index)}
                onClick={() => onSelectAccount(account.id)}
              >
                <span>{account.label}</span>
                <small>{account.email}</small>
                <span className="account-meta">
                  <strong>{accountStatusLabel(account.status, t)}</strong>
                  <span>{localeFormat.unreadCount(account.unreadCount)}</span>
                </span>
                <small className="account-sync-time">
                  {lastSyncLabel(account, localeFormat, t)}
                </small>
              </button>
              <button
                className={
                  accountNeedsAttention(account)
                    ? "account-status-command account-status-command--attention"
                    : "account-status-command"
                }
                type="button"
                onClick={(event) =>
                  onOpenAccountStatus(account.id, event.currentTarget)
                }
              >
                {accountNeedsAttention(account)
                  ? t("mail.account.actions.reconnect")
                  : t("mail.account.actions.details")}
              </button>
            </div>
          ))}
        </StatusList>
      </section>

      <nav className="sidebar-section" aria-label={t("app.sections.folders")}>
        <h2>{t("app.sections.folders")}</h2>
        <StatusList
          status={state.loadState.folders}
          loadingLabel={t("mail.states.foldersLoading")}
          errorLabel={t("mail.states.foldersError")}
          empty={visibleFolders.length === 0}
          emptyLabel={t("mail.states.foldersEmpty")}
          onRetry={onRetry}
        >
          <button
            className={!state.selectedFolderId ? "folder is-active" : "folder"}
            type="button"
            aria-label={t("mail.folders.rootAria", {
              name: state.unifiedInbox
                ? t("mail.folders.unifiedInbox")
                : t("mail.folders.allMail"),
              unread: localeFormat.count(unreadCountForScope(state)),
            })}
            ref={(element) => onFolderButtonRef(0, element)}
            onKeyDown={(event) => onFolderKeyDown(event, 0)}
            onClick={() => onSelectFolder(null)}
          >
            <span>
              {state.unifiedInbox
                ? t("mail.folders.unifiedInbox")
                : t("mail.folders.allMail")}
            </span>
            <span>{localeFormat.count(unreadCountForScope(state))}</span>
          </button>
          {visibleFolders.map((folder, index) => (
            <button
              className={
                folder.id === state.selectedFolderId
                  ? "folder is-active"
                  : "folder"
              }
              type="button"
              key={folder.id}
              aria-label={t("mail.folders.aria", {
                name: folder.name,
                total: localeFormat.count(folder.totalCount),
                unread: localeFormat.count(folder.unreadCount),
              })}
              ref={(element) => onFolderButtonRef(index + 1, element)}
              onKeyDown={(event) => onFolderKeyDown(event, index + 1)}
              onClick={() => onSelectFolder(folder.id)}
            >
              <span>{folder.name}</span>
              <span>{localeFormat.count(folder.unreadCount)}</span>
            </button>
          ))}
        </StatusList>
      </nav>

      <section
        className="sidebar-section sidebar-section--footer"
        aria-label={t("app.sections.app")}
      >
        <button
          className="settings-button"
          type="button"
          onClick={(event) => onOpenSettings(event.currentTarget)}
        >
          <Settings aria-hidden="true" size={16} />
          {t("settings.section")}
        </button>
      </section>
    </aside>
  );
}

function unreadCountForScope(state: MailShellState) {
  const visibleFolders = foldersForScope(state);

  if (state.selectedFolderId) {
    return (
      visibleFolders.find((folder) => folder.id === state.selectedFolderId)
        ?.unreadCount ?? 0
    );
  }

  if (state.unifiedInbox) {
    return visibleFolders
      .filter((folder) => folder.kind === "inbox")
      .reduce((total, folder) => total + folder.unreadCount, 0);
  }

  return visibleFolders.reduce((total, folder) => total + folder.unreadCount, 0);
}
