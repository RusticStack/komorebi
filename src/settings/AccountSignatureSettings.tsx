import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { SafeEmailBody } from "../mail/rendering/SafeEmailBody";
import { sanitizeSignatureHtml } from "../mail/signatures/signatureHtml";
import type {
  AccountAuthKind,
  AccountAuthStatusView,
  AccountConnectionTestResponse,
  AccountManagementView,
  AccountAuthStatusRequest,
  AccountProviderKind,
  AccountReauthRequest,
  AccountRemovalRequest,
  AccountRemovalResponse,
  AccountSecurityMode,
  AccountSetupRequest,
  AccountSetupResponse,
} from "../mail/shell/mailShellState";

export type SignatureAccountOption = {
  id: string;
  label: string;
  email: string;
  signatureHtml: string | null;
};

export type AccountSignatureSettingsProps = {
  accounts: SignatureAccountOption[];
  initialAccountId?: string | null;
  onSaveSignature: (
    accountId: string,
    signatureHtml: string | null,
  ) => void | Promise<void>;
  onAccountChanged?: () => void;
  onCreateAccount?: (
    request: AccountSetupRequest,
  ) => Promise<AccountSetupResponse>;
  onReauthAccount?: (
    request: AccountReauthRequest,
  ) => Promise<AccountSetupResponse>;
  onRemoveAccount?: (
    request: AccountRemovalRequest,
  ) => Promise<AccountRemovalResponse>;
  onTestAccountConnection?: (
    request: AccountSetupRequest,
  ) => Promise<AccountConnectionTestResponse>;
  onGetAccountAuthStatus?: (
    request: AccountAuthStatusRequest,
  ) => Promise<AccountAuthStatusView>;
  onClose?: () => void;
};

type AccountFormState = {
  providerKind: AccountProviderKind;
  displayEmail: string;
  displayName: string;
  username: string;
  authKind: AccountAuthKind;
  secret: string;
  imapHost: string;
  imapPort: string;
  imapSecurity: AccountSecurityMode;
  smtpHost: string;
  smtpPort: string;
  smtpSecurity: AccountSecurityMode;
};

const defaultAccountForm: AccountFormState = {
  providerKind: "generic_imap_smtp",
  displayEmail: "",
  displayName: "",
  username: "",
  authKind: "app_password",
  secret: "",
  imapHost: "",
  imapPort: "993",
  imapSecurity: "tls",
  smtpHost: "",
  smtpPort: "587",
  smtpSecurity: "starttls",
};

export function AccountSignatureSettings({
  accounts,
  initialAccountId,
  onSaveSignature,
  onAccountChanged,
  onCreateAccount,
  onReauthAccount,
  onRemoveAccount,
  onTestAccountConnection,
  onGetAccountAuthStatus,
  onClose,
}: AccountSignatureSettingsProps) {
  const { t } = useTranslation();
  const [selectedAccountId, setSelectedAccountId] = useState(
    initialAccountId ?? accounts[0]?.id ?? "",
  );
  const [draftsByAccount, setDraftsByAccount] = useState<Record<string, string>>(
    () =>
      Object.fromEntries(
        accounts.map((account) => [account.id, account.signatureHtml ?? ""]),
      ),
  );
  const [accountForm, setAccountForm] = useState<AccountFormState>(
    defaultAccountForm,
  );
  const [reauthSecret, setReauthSecret] = useState("");
  const [deleteLocalCache, setDeleteLocalCache] = useState(true);
  const [accountNotice, setAccountNotice] = useState<string | null>(null);
  const [accountError, setAccountError] = useState<string | null>(null);
  const [authStatus, setAuthStatus] = useState<AccountAuthStatusView | null>(
    null,
  );
  const selectedAccount =
    accounts.find((account) => account.id === selectedAccountId) ?? null;
  const draftHtml = selectedAccount
    ? draftsByAccount[selectedAccount.id] ?? selectedAccount.signatureHtml ?? ""
    : "";
  const sanitized = useMemo(
    () => sanitizeSignatureHtml(draftHtml),
    [draftHtml],
  );

  return (
    <section className="settings-panel" aria-label={t("settings.aria.panel")}>
      <header className="settings-panel__header">
        <div>
          <p className="eyebrow">{t("settings.section")}</p>
          <h2>{t("settings.heading")}</h2>
        </div>
        {onClose ? (
          <button type="button" onClick={onClose}>
            {t("common.actions.close")}
          </button>
        ) : null}
      </header>

      <section
        className="account-settings"
        aria-label={t("settings.accounts.heading")}
      >
        <h3>{t("settings.accounts.heading")}</h3>
        <form
          className="account-form"
          onSubmit={(event) => {
            event.preventDefault();
            void handleCreateAccount();
          }}
        >
          <label>
            <span>{t("settings.accounts.fields.provider")}</span>
            <select
              value={accountForm.providerKind}
              onChange={(event) => {
                const providerKind = event.target.value as AccountProviderKind;
                setAccountForm((current) => ({
                  ...current,
                  providerKind,
                  authKind:
                    providerKind === "generic_imap_smtp"
                      ? "app_password"
                      : "oauth2",
                }));
              }}
            >
              <option value="generic_imap_smtp">
                {t("settings.accounts.providers.generic_imap_smtp")}
              </option>
              <option value="gmail">
                {t("settings.accounts.providers.gmail")}
              </option>
              <option value="microsoft">
                {t("settings.accounts.providers.microsoft")}
              </option>
            </select>
          </label>

          <label>
            <span>{t("settings.accounts.fields.displayEmail")}</span>
            <input
              value={accountForm.displayEmail}
              onChange={(event) =>
                updateAccountForm({ displayEmail: event.target.value })
              }
            />
          </label>

          <label>
            <span>{t("settings.accounts.fields.displayName")}</span>
            <input
              value={accountForm.displayName}
              onChange={(event) =>
                updateAccountForm({ displayName: event.target.value })
              }
            />
          </label>

          <label>
            <span>{t("settings.accounts.fields.username")}</span>
            <input
              value={accountForm.username}
              onChange={(event) =>
                updateAccountForm({ username: event.target.value })
              }
            />
          </label>

          {accountForm.providerKind === "generic_imap_smtp" ? (
            <>
              <label>
                <span>{t("settings.accounts.fields.authKind")}</span>
                <select
                  value={accountForm.authKind}
                  onChange={(event) =>
                    updateAccountForm({
                      authKind: event.target.value as AccountAuthKind,
                    })
                  }
                >
                  <option value="app_password">
                    {t("settings.accounts.auth.app_password")}
                  </option>
                  <option value="manual_password">
                    {t("settings.accounts.auth.manual_password")}
                  </option>
                </select>
              </label>

              <fieldset className="account-server-grid">
                <legend>{t("settings.accounts.sections.incoming")}</legend>
                <label>
                  <span>{t("settings.accounts.fields.imapHost")}</span>
                  <input
                    value={accountForm.imapHost}
                    onChange={(event) =>
                      updateAccountForm({ imapHost: event.target.value })
                    }
                  />
                </label>
                <label>
                  <span>{t("settings.accounts.fields.imapPort")}</span>
                  <input
                    inputMode="numeric"
                    value={accountForm.imapPort}
                    onChange={(event) =>
                      updateAccountForm({ imapPort: event.target.value })
                    }
                  />
                </label>
                <label>
                  <span>{t("settings.accounts.fields.imapSecurity")}</span>
                  <select
                    value={accountForm.imapSecurity}
                    onChange={(event) =>
                      updateAccountForm({
                        imapSecurity: event.target.value as AccountSecurityMode,
                      })
                    }
                  >
                    {securityOptions(t)}
                  </select>
                </label>
              </fieldset>

              <fieldset className="account-server-grid">
                <legend>{t("settings.accounts.sections.outgoing")}</legend>
                <label>
                  <span>{t("settings.accounts.fields.smtpHost")}</span>
                  <input
                    value={accountForm.smtpHost}
                    onChange={(event) =>
                      updateAccountForm({ smtpHost: event.target.value })
                    }
                  />
                </label>
                <label>
                  <span>{t("settings.accounts.fields.smtpPort")}</span>
                  <input
                    inputMode="numeric"
                    value={accountForm.smtpPort}
                    onChange={(event) =>
                      updateAccountForm({ smtpPort: event.target.value })
                    }
                  />
                </label>
                <label>
                  <span>{t("settings.accounts.fields.smtpSecurity")}</span>
                  <select
                    value={accountForm.smtpSecurity}
                    onChange={(event) =>
                      updateAccountForm({
                        smtpSecurity: event.target.value as AccountSecurityMode,
                      })
                    }
                  >
                    {securityOptions(t)}
                  </select>
                </label>
              </fieldset>
            </>
          ) : null}

          <label>
            <span>
              {accountForm.providerKind === "generic_imap_smtp"
                ? t("settings.accounts.fields.passwordSecret")
                : t("settings.accounts.fields.oauthSecret")}
            </span>
            <input
              type="password"
              value={accountForm.secret}
              onChange={(event) =>
                updateAccountForm({ secret: event.target.value })
              }
            />
          </label>

          <div className="settings-panel__actions">
            <button
              type="button"
              onClick={() => void handleTestAccount()}
              disabled={!onTestAccountConnection}
            >
              {t("settings.accounts.actions.test")}
            </button>
            <button type="submit" disabled={!onCreateAccount}>
              {t("settings.accounts.actions.add")}
            </button>
          </div>
        </form>

        {selectedAccount ? (
          <form
            className="account-form account-form--compact"
            onSubmit={(event) => {
              event.preventDefault();
              void handleReauthAccount();
            }}
          >
            <h4>{t("settings.accounts.sections.selected")}</h4>
            <p>
              {selectedAccount.label} - {selectedAccount.email}
            </p>
            <label>
              <span>{t("settings.accounts.fields.reauthSecret")}</span>
              <input
                type="password"
                value={reauthSecret}
                onChange={(event) => setReauthSecret(event.target.value)}
              />
            </label>
            <label className="checkbox-row">
              <input
                type="checkbox"
                checked={deleteLocalCache}
                onChange={(event) => setDeleteLocalCache(event.target.checked)}
              />
              <span>{t("settings.accounts.fields.deleteLocalCache")}</span>
            </label>
            <div className="settings-panel__actions">
              <button
                type="button"
                onClick={() => void handleLoadAuthStatus()}
                disabled={!onGetAccountAuthStatus}
              >
                {t("settings.accounts.actions.status")}
              </button>
              <button type="submit" disabled={!onReauthAccount}>
                {t("settings.accounts.actions.reauth")}
              </button>
              <button
                type="button"
                onClick={() => void handleRemoveAccount()}
                disabled={!onRemoveAccount}
              >
                {t("settings.accounts.actions.remove")}
              </button>
            </div>
          </form>
        ) : (
          <p className="empty-note">{t("mail.states.noAccounts")}</p>
        )}

        {accountNotice ? (
          <p className="settings-notice" role="status">
            {accountNotice}
          </p>
        ) : null}
        {accountError ? (
          <p className="settings-error" role="alert">
            {accountError}
          </p>
        ) : null}
        {authStatus ? (
          <p className="settings-notice" role="status">
              {t("settings.accounts.statusSummary", {
                status: t(`mail.account.status.${statusForUi(authStatus.authStatus)}`),
                credential:
                  authStatus.credentialPresent
                  ? t("settings.accounts.credential.present")
                  : t("settings.accounts.credential.missing"),
            })}
          </p>
        ) : null}
      </section>

      {accounts.length === 0 ? null : (
        <form
          className="signature-settings"
          onSubmit={(event) => {
            event.preventDefault();
            if (!selectedAccount) {
              return;
            }

            void onSaveSignature(
              selectedAccount.id,
              sanitized.html.trim() || null,
            );
          }}
        >
          <label>
            <span>{t("settings.fields.account")}</span>
            <select
              value={selectedAccountId}
              onChange={(event) => setSelectedAccountId(event.target.value)}
            >
              {accounts.map((account) => (
                <option value={account.id} key={account.id}>
                  {account.label} - {account.email}
                </option>
              ))}
            </select>
          </label>

          <label>
            <span>{t("settings.fields.signatureHtml")}</span>
            <textarea
              value={draftHtml}
              onChange={(event) => {
                if (!selectedAccount) {
                  return;
                }

                setDraftsByAccount((current) => ({
                  ...current,
                  [selectedAccount.id]: event.target.value,
                }));
              }}
              rows={7}
            />
          </label>

          <section
            className="signature-preview"
            aria-label={t("settings.aria.preview")}
          >
            <SafeEmailBody
              bodyHtml={sanitized.html}
              bodyText={sanitized.plainText}
            />
          </section>

          <div className="settings-panel__actions">
            <button
              type="button"
              onClick={() => {
                if (!selectedAccount) {
                  return;
                }

                setDraftsByAccount((current) => ({
                  ...current,
                  [selectedAccount.id]: "",
                }));
              }}
            >
              {t("settings.actions.clear")}
            </button>
            <button type="submit">{t("settings.actions.saveSignature")}</button>
          </div>
        </form>
      )}
    </section>
  );

  function updateAccountForm(update: Partial<AccountFormState>) {
    setAccountForm((current) => ({ ...current, ...update }));
  }

  function buildSetupRequest(): AccountSetupRequest {
    const isGeneric = accountForm.providerKind === "generic_imap_smtp";

    return {
      providerKind: accountForm.providerKind,
      displayEmail: accountForm.displayEmail,
      displayName: accountForm.displayName.trim() || null,
      username: accountForm.username,
      authKind: isGeneric ? accountForm.authKind : "oauth2",
      secret: accountForm.secret,
      imapHost: isGeneric ? accountForm.imapHost : null,
      imapPort: isGeneric ? Number(accountForm.imapPort) || null : null,
      imapSecurity: isGeneric ? accountForm.imapSecurity : null,
      smtpHost: isGeneric ? accountForm.smtpHost : null,
      smtpPort: isGeneric ? Number(accountForm.smtpPort) || null : null,
      smtpSecurity: isGeneric ? accountForm.smtpSecurity : null,
    };
  }

  async function handleTestAccount() {
    if (!onTestAccountConnection) {
      setAccountError(t("settings.accounts.errors.testUnavailable"));
      return;
    }
    await runAccountAction(async () => {
      await onTestAccountConnection(buildSetupRequest());
      setAccountNotice(t("settings.accounts.notices.testReady"));
    });
  }

  async function handleCreateAccount() {
    if (!onCreateAccount) {
      setAccountError(t("settings.accounts.errors.createUnavailable"));
      return;
    }
    await runAccountAction(async () => {
      const response = await onCreateAccount(buildSetupRequest());
      setSelectedAccountId(response.account.id);
      setAccountNotice(
        t("settings.accounts.notices.created", {
          email: response.account.displayEmail,
        }),
      );
      setAccountForm(defaultAccountForm);
      onAccountChanged?.();
    });
  }

  async function handleReauthAccount() {
    if (!onReauthAccount || !selectedAccount) {
      setAccountError(t("settings.accounts.errors.reauthUnavailable"));
      return;
    }
    await runAccountAction(async () => {
      await onReauthAccount({
        accountId: selectedAccount.id,
        secret: reauthSecret,
      });
      setReauthSecret("");
      setAccountNotice(t("settings.accounts.notices.reauthed"));
      onAccountChanged?.();
    });
  }

  async function handleRemoveAccount() {
    if (!onRemoveAccount || !selectedAccount) {
      setAccountError(t("settings.accounts.errors.removeUnavailable"));
      return;
    }
    await runAccountAction(async () => {
      await onRemoveAccount({
        accountId: selectedAccount.id,
        deleteLocalCache,
      });
      setSelectedAccountId(
        accounts.find((account) => account.id !== selectedAccount.id)?.id ??
          "",
      );
      setAccountNotice(t("settings.accounts.notices.removed"));
      onAccountChanged?.();
    });
  }

  async function handleLoadAuthStatus() {
    if (!onGetAccountAuthStatus || !selectedAccount) {
      setAccountError(t("settings.accounts.errors.statusUnavailable"));
      return;
    }
    await runAccountAction(async () => {
      setAuthStatus(
        await onGetAccountAuthStatus({
          accountId: selectedAccount.id,
        }),
      );
    });
  }

  async function runAccountAction(action: () => Promise<void>) {
    setAccountError(null);
    setAccountNotice(null);
    setAuthStatus(null);
    try {
      await action();
    } catch (error) {
      setAccountError(formatAccountError(error));
    }
  }
}

function securityOptions(t: ReturnType<typeof useTranslation>["t"]) {
  return (
    <>
      <option value="tls">{t("settings.accounts.security.tls")}</option>
      <option value="starttls">{t("settings.accounts.security.starttls")}</option>
    </>
  );
}

function statusForUi(status: AccountManagementView["authStatus"]) {
  return status === "reconnect_needed" ? "reconnect-needed" : status;
}

function formatAccountError(error: unknown) {
  if (error instanceof Error) {
    return error.message;
  }

  if (
    error &&
    typeof error === "object" &&
    "reason" in error &&
    typeof error.reason === "string"
  ) {
    return error.reason;
  }

  return String(error);
}
