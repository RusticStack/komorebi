import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { useLocaleFormatters } from "../../i18n";
import type { MailAccount } from "./mailShellState";
import {
  accountStatusLabel,
  formatAccountError,
  lastSyncValue,
  statusMessageFallback,
} from "./accountStatusDisplay";

export function AccountStatusDrawer({
  account,
  localeFormat,
  onClose,
  onReconnect,
  requiresSecret,
}: {
  account: MailAccount;
  localeFormat: ReturnType<typeof useLocaleFormatters>;
  onClose: () => void;
  onReconnect: (secret: string) => Promise<void>;
  requiresSecret: boolean;
}) {
  const { t } = useTranslation();
  const [secret, setSecret] = useState("");
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const statusMessage =
    account.statusMessage?.trim() || statusMessageFallback(account, t);

  async function handleReconnect() {
    setNotice(null);
    setError(null);
    if (requiresSecret && !secret.trim()) {
      setError(t("mail.account.statusDrawer.secretRequired"));
      return;
    }

    setRunning(true);
    try {
      await onReconnect(secret);
      setSecret("");
      setNotice(t("mail.account.statusDrawer.reconnectStarted"));
    } catch (reconnectError) {
      setError(formatAccountError(reconnectError));
    } finally {
      setRunning(false);
    }
  }

  return (
    <section
      className="settings-panel account-status-panel"
      aria-label={t("mail.account.statusDrawer.aria")}
    >
      <header className="settings-panel__header">
        <div>
          <p className="eyebrow">{t("mail.account.statusDrawer.eyebrow")}</p>
          <h2>{account.label}</h2>
        </div>
        <button type="button" onClick={onClose}>
          {t("common.actions.close")}
        </button>
      </header>

      <section
        className="account-status-detail"
        aria-label={t("mail.account.statusDrawer.detailAria")}
      >
        <dl>
          <div>
            <dt>{t("mail.account.statusDrawer.status")}</dt>
            <dd>{accountStatusLabel(account.status, t)}</dd>
          </div>
          <div>
            <dt>{t("mail.account.statusDrawer.lastSuccessfulSync")}</dt>
            <dd>{lastSyncValue(account, localeFormat, t)}</dd>
          </div>
          <div>
            <dt>{t("mail.account.statusDrawer.lastStatusUpdate")}</dt>
            <dd>
              {account.statusUpdatedAt
                ? localeFormat.relativeDateTime(account.statusUpdatedAt)
                : t("mail.account.statusDrawer.statusUpdateUnavailable")}
            </dd>
          </div>
        </dl>
        <p>{statusMessage}</p>
      </section>

      <form
        className="account-form account-form--compact"
        onSubmit={(event) => {
          event.preventDefault();
          void handleReconnect();
        }}
      >
        <h3>{t("mail.account.statusDrawer.reconnectHeading")}</h3>
        <p className="account-form__hint">
          {requiresSecret
            ? t("mail.account.statusDrawer.genericReconnectHint")
            : t("mail.account.statusDrawer.oauthReconnectHint")}
        </p>
        {requiresSecret ? (
          <label>
            <span>{t("settings.accounts.fields.reauthSecret")}</span>
            <input
              data-autofocus="true"
              type="password"
              value={secret}
              onChange={(event) => setSecret(event.target.value)}
            />
          </label>
        ) : null}
        <div className="settings-panel__actions">
          <button type="submit" disabled={running}>
            {running
              ? t("mail.account.statusDrawer.reconnecting")
              : t("mail.account.actions.reconnect")}
          </button>
        </div>
      </form>

      {notice ? (
        <p className="settings-notice" role="status">
          {notice}
        </p>
      ) : null}
      {error ? (
        <p className="settings-error" role="alert">
          {error}
        </p>
      ) : null}
    </section>
  );
}
