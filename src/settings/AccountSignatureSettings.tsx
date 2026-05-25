import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { SafeEmailBody } from "../mail/rendering/SafeEmailBody";
import { sanitizeSignatureHtml } from "../mail/signatures/signatureHtml";
import type {
  AccountAuthKind,
  AccountAuthStatusView,
  AccountConnectionTestResponse,
  AccountLocalPreferences,
  AccountManagementView,
  AccountAuthStatusRequest,
  AccountProviderKind,
  AccountSettingsUpdateRequest,
  AccountSettingsUpdateResponse,
  OAuthAccountProviderKind,
  OAuthAccountReauthRequest,
  OAuthAccountSetupRequest,
  AccountReauthRequest,
  AccountRemovalRequest,
  AccountRemovalResponse,
  AccountSecurityMode,
  AccountSetupRequest,
  AccountSetupResponse,
  DiagnosticsExportRequest,
  DiagnosticsExportResponse,
  DiagnosticsHealthEvent,
  DiagnosticsHealthView,
  LoadStatus,
  UiLanguagePreference,
} from "../mail/shell/mailShellState";

export type SignatureAccountOption = {
  id: string;
  label: string;
  email: string;
  displayName?: string | null;
  providerKind?: AccountProviderKind;
  authKind?: AccountAuthKind;
  signatureHtml: string | null;
  localPreferences?: AccountLocalPreferences;
};

export type AccountSignatureSettingsProps = {
  accounts: SignatureAccountOption[];
  initialAccountId?: string | null;
  languagePreference?: UiLanguagePreference | null;
  languagePreferenceStatus?: LoadStatus;
  onChangeLanguagePreference?: (
    language: UiLanguagePreference | null,
  ) => Promise<void>;
  onSaveSignature?: (
    accountId: string,
    signatureHtml: string | null,
  ) => void | Promise<void>;
  onSaveAccountSettings?: (
    request: AccountSettingsUpdateRequest,
  ) => Promise<AccountSettingsUpdateResponse>;
  onAccountChanged?: () => void;
  onCreateAccount?: (
    request: AccountSetupRequest,
  ) => Promise<AccountSetupResponse>;
  onReauthAccount?: (
    request: AccountReauthRequest,
  ) => Promise<AccountSetupResponse>;
  onStartOAuthAccountSetup?: (
    request: OAuthAccountSetupRequest,
  ) => Promise<AccountSetupResponse>;
  onStartOAuthAccountReauth?: (
    request: OAuthAccountReauthRequest,
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
  onExportDiagnostics?: (
    request: DiagnosticsExportRequest,
  ) => Promise<DiagnosticsExportResponse>;
  onLoadDiagnosticsHealth?: () => Promise<DiagnosticsHealthView>;
  onClose?: () => void;
};

type AccountFormState = {
  providerKind: AccountProviderKind;
  displayEmail: string;
  displayName: string;
  username: string;
  authKind: AccountAuthKind;
  secret: string;
  oauthClientId: string;
  imapHost: string;
  imapPort: string;
  imapSecurity: AccountSecurityMode;
  smtpHost: string;
  smtpPort: string;
  smtpSecurity: AccountSecurityMode;
};
type AccountSetupStep = "identity" | "connection" | "finish";

const defaultAccountForm: AccountFormState = {
  providerKind: "generic_imap_smtp",
  displayEmail: "",
  displayName: "",
  username: "",
  authKind: "app_password",
  secret: "",
  oauthClientId: "",
  imapHost: "",
  imapPort: "993",
  imapSecurity: "tls",
  smtpHost: "",
  smtpPort: "587",
  smtpSecurity: "starttls",
};

const defaultLocalPreferences: AccountLocalPreferences = {
  includeInUnifiedInbox: true,
};

export function AccountSignatureSettings({
  accounts,
  initialAccountId,
  languagePreference = null,
  languagePreferenceStatus = "ready",
  onChangeLanguagePreference,
  onSaveSignature,
  onSaveAccountSettings,
  onAccountChanged,
  onCreateAccount,
  onReauthAccount,
  onStartOAuthAccountSetup,
  onStartOAuthAccountReauth,
  onRemoveAccount,
  onTestAccountConnection,
  onGetAccountAuthStatus,
  onExportDiagnostics,
  onLoadDiagnosticsHealth,
  onClose,
}: AccountSignatureSettingsProps) {
  const { t, i18n } = useTranslation();
  const [selectedAccountId, setSelectedAccountId] = useState(
    initialAccountId ?? accounts[0]?.id ?? "",
  );
  const [draftsByAccount, setDraftsByAccount] = useState<Record<string, string>>(
    {},
  );
  const [displayNamesByAccount, setDisplayNamesByAccount] = useState<
    Record<string, string>
  >({});
  const [preferencesByAccount, setPreferencesByAccount] = useState<
    Record<string, AccountLocalPreferences>
  >({});
  const [accountForm, setAccountForm] = useState<AccountFormState>(
    defaultAccountForm,
  );
  const [accountSetupStep, setAccountSetupStep] =
    useState<AccountSetupStep>("identity");
  const [reauthSecret, setReauthSecret] = useState("");
  const [removalCacheChoice, setRemovalCacheChoice] = useState<
    "delete" | "retain"
  >("delete");
  const [removalConfirmed, setRemovalConfirmed] = useState(false);
  const [accountNotice, setAccountNotice] = useState<string | null>(null);
  const [accountError, setAccountError] = useState<string | null>(null);
  const [languageNoticeKey, setLanguageNoticeKey] = useState<string | null>(
    null,
  );
  const [languageError, setLanguageError] = useState<string | null>(null);
  const [languageSaving, setLanguageSaving] = useState(false);
  const [includeSyntheticFixtures, setIncludeSyntheticFixtures] =
    useState(false);
  const [diagnosticsExporting, setDiagnosticsExporting] = useState(false);
  const [diagnosticsNotice, setDiagnosticsNotice] = useState<string | null>(
    null,
  );
  const [diagnosticsError, setDiagnosticsError] = useState<string | null>(null);
  const [diagnosticsHealth, setDiagnosticsHealth] =
    useState<DiagnosticsHealthView | null>(null);
  const [diagnosticsHealthLoading, setDiagnosticsHealthLoading] =
    useState(false);
  const [diagnosticsHealthError, setDiagnosticsHealthError] = useState<
    string | null
  >(null);
  const [authStatus, setAuthStatus] = useState<AccountAuthStatusView | null>(
    null,
  );
  const activeSelectedAccountId = accounts.some(
    (account) => account.id === selectedAccountId,
  )
    ? selectedAccountId
    : accounts.find((account) => account.id === initialAccountId)?.id ??
      accounts[0]?.id ??
      "";
  const selectedAccount =
    accounts.find((account) => account.id === activeSelectedAccountId) ?? null;
  const selectedAccountUsesOAuth =
    selectedAccount?.authKind === "oauth2" &&
    selectedAccount.providerKind !== "generic_imap_smtp";
  const draftHtml = selectedAccount
    ? draftsByAccount[selectedAccount.id] ?? selectedAccount.signatureHtml ?? ""
    : "";
  const selectedDisplayName = selectedAccount
    ? displayNamesByAccount[selectedAccount.id] ??
      selectedAccount.displayName ??
      ""
    : "";
  const selectedPreferences = selectedAccount
    ? preferencesByAccount[selectedAccount.id] ??
      selectedAccount.localPreferences ??
      defaultLocalPreferences
    : defaultLocalPreferences;
  const sanitized = useMemo(
    () => sanitizeSignatureHtml(draftHtml),
    [draftHtml],
  );
  const locale = i18n.resolvedLanguage ?? i18n.language;
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(locale),
    [locale],
  );
  const dateFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(locale, {
        dateStyle: "medium",
        timeStyle: "short",
      }),
    [locale],
  );

  useEffect(() => {
    if (!onLoadDiagnosticsHealth) {
      return;
    }

    let cancelled = false;
    void (async () => {
      setDiagnosticsHealthLoading(true);
      setDiagnosticsHealthError(null);
      try {
        const health = await onLoadDiagnosticsHealth();
        if (cancelled) {
          return;
        }
        setDiagnosticsHealth(health);
      } catch (error) {
        if (cancelled) {
          return;
        }
        setDiagnosticsHealthError(formatAccountError(error));
      } finally {
        if (!cancelled) {
          setDiagnosticsHealthLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [onLoadDiagnosticsHealth]);

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
        className="app-preferences"
        aria-label={t("settings.language.heading")}
      >
        <h3>{t("settings.language.heading")}</h3>
        <label>
          <span>{t("settings.language.label")}</span>
          <select
            value={languagePreference ?? "system"}
            disabled={
              !onChangeLanguagePreference ||
              languagePreferenceStatus === "loading" ||
              languageSaving
            }
            onChange={(event) => {
              const value = event.target.value;
              void handleLanguagePreferenceChange(
                value === "system" ? null : (value as UiLanguagePreference),
              );
            }}
          >
            <option value="system">{t("settings.language.system")}</option>
            <option value="en">{t("settings.language.en")}</option>
            <option value="pt-PT">{t("settings.language.ptPT")}</option>
          </select>
        </label>
        {languagePreferenceStatus === "loading" ? (
          <p className="settings-notice" role="status">
            {t("settings.language.loading")}
          </p>
        ) : null}
        {languageSaving ? (
          <p className="settings-notice" role="status">
            {t("settings.language.saving")}
          </p>
        ) : null}
        {languageNoticeKey ? (
          <p className="settings-notice" role="status">
            {t(languageNoticeKey)}
          </p>
        ) : null}
        {languagePreferenceStatus === "error" ? (
          <p className="settings-error" role="alert">
            {t("settings.language.loadError")}
          </p>
        ) : null}
        {languageError ? (
          <p className="settings-error" role="alert">
            {languageError}
          </p>
        ) : null}
        {!onChangeLanguagePreference ? (
          <p className="account-form__hint">
            {t("settings.language.unavailable")}
          </p>
        ) : null}
      </section>

      <section
        className="diagnostics-settings"
        aria-label={t("settings.diagnostics.heading")}
      >
        <h3>{t("settings.diagnostics.heading")}</h3>
        <p className="account-form__hint">
          {t("settings.diagnostics.copy")}
        </p>
        <section
          className="diagnostics-health"
          aria-label={t("settings.diagnostics.health.heading")}
        >
          <div className="diagnostics-health__header">
            <div>
              <h4>{t("settings.diagnostics.health.heading")}</h4>
              {diagnosticsHealth ? (
                <p>
                  {t("settings.diagnostics.health.generated", {
                    time: dateFormatter.format(
                      new Date(diagnosticsHealth.generatedAtUnixMs),
                    ),
                  })}
                </p>
              ) : null}
            </div>
            <button
              type="button"
              onClick={() => void handleRefreshDiagnosticsHealth()}
              disabled={!onLoadDiagnosticsHealth || diagnosticsHealthLoading}
            >
              {diagnosticsHealthLoading
                ? t("settings.diagnostics.health.loading")
                : t("settings.diagnostics.health.refresh")}
            </button>
          </div>
          {!onLoadDiagnosticsHealth ? (
            <p className="account-form__hint">
              {t("settings.diagnostics.health.unavailable")}
            </p>
          ) : null}
          {diagnosticsHealthError ? (
            <p className="settings-error" role="alert">
              {diagnosticsHealthError}
            </p>
          ) : null}
          {diagnosticsHealth ? renderDiagnosticsHealth() : null}
        </section>
        <label className="checkbox-row">
          <input
            type="checkbox"
            checked={includeSyntheticFixtures}
            onChange={(event) =>
              setIncludeSyntheticFixtures(event.target.checked)
            }
          />
          <span>{t("settings.diagnostics.includeSynthetic")}</span>
        </label>
        <div className="settings-panel__actions">
          <button
            type="button"
            onClick={() => void handleExportDiagnostics()}
            disabled={!onExportDiagnostics || diagnosticsExporting}
          >
            {diagnosticsExporting
              ? t("settings.diagnostics.exporting")
              : t("settings.diagnostics.export")}
          </button>
        </div>
        {!onExportDiagnostics ? (
          <p className="account-form__hint">
            {t("settings.diagnostics.unavailable")}
          </p>
        ) : null}
        {diagnosticsNotice ? (
          <p className="settings-notice" role="status">
            {diagnosticsNotice}
          </p>
        ) : null}
        {diagnosticsError ? (
          <p className="settings-error" role="alert">
            {diagnosticsError}
          </p>
        ) : null}
      </section>

      <section
        className="account-settings"
        aria-label={t("settings.accounts.heading")}
      >
        <h3>{t("settings.accounts.heading")}</h3>
        <form
          className="account-form account-setup-wizard"
          onSubmit={(event) => {
            event.preventDefault();
            void handleCreateAccount();
          }}
        >
          <header className="account-setup-wizard__header">
            <div>
              <p className="eyebrow">{t("settings.accounts.wizard.eyebrow")}</p>
              <h4>{t("settings.accounts.wizard.heading")}</h4>
              <p className="account-form__hint">
                {t("settings.accounts.wizard.copy")}
              </p>
            </div>
            <ol className="account-setup-wizard__steps">
              {(["identity", "connection", "finish"] as const).map((step) => (
                <li
                  className={
                    accountSetupStep === step
                      ? "account-setup-wizard__step is-active"
                      : "account-setup-wizard__step"
                  }
                  key={step}
                >
                  {t(`settings.accounts.wizard.steps.${step}`)}
                </li>
              ))}
            </ol>
          </header>

          <section className="account-setup-wizard__card">
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
                  secret: "",
                  oauthClientId: "",
                }));
                setAccountSetupStep(
                  providerKind === "generic_imap_smtp"
                    ? "connection"
                    : "finish",
                );
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
              onBlur={() => handleDetectAccountSettings({ silent: true })}
              onChange={(event) => {
                updateAccountForm({ displayEmail: event.target.value });
                setAccountSetupStep("identity");
              }}
            />
          </label>
          <button
            className="account-detect-button"
            type="button"
            onClick={() => handleDetectAccountSettings({ silent: false })}
          >
            {t("settings.accounts.actions.detect")}
          </button>

          <label>
            <span>{t("settings.accounts.fields.displayName")}</span>
            <input
              value={accountForm.displayName}
              onChange={(event) =>
                updateAccountForm({ displayName: event.target.value })
              }
            />
          </label>
          </section>

          <section className="account-setup-wizard__card">

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
              <p className="account-form__hint">
                {t("settings.accounts.compatibility.generic")}
              </p>
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

          {accountForm.providerKind === "generic_imap_smtp" ? (
            <label>
              <span>{t("settings.accounts.fields.passwordSecret")}</span>
              <input
                type="password"
                value={accountForm.secret}
                onChange={(event) =>
                  updateAccountForm({ secret: event.target.value })
                }
              />
            </label>
          ) : (
            <label>
              <span>{t("settings.accounts.fields.oauthClientId")}</span>
              <input
                value={accountForm.oauthClientId}
                onChange={(event) =>
                  updateAccountForm({ oauthClientId: event.target.value })
                }
              />
            </label>
          )}
          </section>

          <div className="settings-panel__actions">
            <button
              type="button"
              onClick={() => void handleTestAccount()}
              disabled={
                !onTestAccountConnection ||
                accountForm.providerKind !== "generic_imap_smtp"
              }
            >
              {t("settings.accounts.actions.test")}
            </button>
            <button
              type="button"
              onClick={() =>
                setAccountSetupStep(
                  accountForm.providerKind === "generic_imap_smtp"
                    ? "connection"
                    : "finish",
                )
              }
            >
              {t("settings.accounts.actions.review")}
            </button>
            <button type="submit" disabled={!canSubmitAccountSetup()}>
              {accountForm.providerKind === "generic_imap_smtp"
                ? t("settings.accounts.actions.add")
                : t("settings.accounts.actions.oauth")}
            </button>
          </div>
        </form>

        {selectedAccount ? (
          <>
            <form
              className="account-form account-form--compact"
              onSubmit={(event) => {
                event.preventDefault();
                void handleSaveAccountSettings();
              }}
            >
              <h4>{t("settings.accounts.sections.preferences")}</h4>
              <p>
                {selectedAccount.label} - {selectedAccount.email}
              </p>
              <label>
                <span>{t("settings.accounts.fields.accountDisplayName")}</span>
                <input
                  value={selectedDisplayName}
                  onChange={(event) => {
                    setDisplayNamesByAccount((current) => ({
                      ...current,
                      [selectedAccount.id]: event.target.value,
                    }));
                  }}
                />
              </label>
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  checked={selectedPreferences.includeInUnifiedInbox}
                  onChange={(event) => {
                    setPreferencesByAccount((current) => ({
                      ...current,
                      [selectedAccount.id]: {
                        includeInUnifiedInbox: event.target.checked,
                      },
                    }));
                  }}
                />
                <span>
                  {t("settings.accounts.fields.includeInUnifiedInbox")}
                </span>
              </label>
              <div className="settings-panel__actions">
                <button type="submit" disabled={!canSaveAccountSettings()}>
                  {t("settings.accounts.actions.saveSettings")}
                </button>
              </div>
            </form>
            <form
              className="account-form account-form--compact"
              onSubmit={(event) => {
                event.preventDefault();
                void handleReauthAccount();
              }}
            >
              <h4>{t("settings.accounts.sections.authentication")}</h4>
              <label>
                <span>
                  {selectedAccountUsesOAuth
                    ? t("settings.accounts.fields.oauthClientId")
                    : t("settings.accounts.fields.reauthSecret")}
                </span>
                <input
                  type={selectedAccountUsesOAuth ? "text" : "password"}
                  value={reauthSecret}
                  onChange={(event) => setReauthSecret(event.target.value)}
                />
              </label>
              <fieldset className="account-removal-options">
                <legend>{t("settings.accounts.removal.heading")}</legend>
                <p className="account-form__hint">
                  {t("settings.accounts.removal.copy")}
                </p>
                <label className="checkbox-row">
                  <input
                    type="radio"
                    name="account-removal-cache"
                    checked={removalCacheChoice === "delete"}
                    onChange={() => {
                      setRemovalCacheChoice("delete");
                      setRemovalConfirmed(false);
                    }}
                  />
                  <span>{t("settings.accounts.removal.deleteCache")}</span>
                </label>
                <p className="account-form__hint">
                  {t("settings.accounts.removal.deleteCacheHelp")}
                </p>
                <label className="checkbox-row">
                  <input
                    type="radio"
                    name="account-removal-cache"
                    checked={removalCacheChoice === "retain"}
                    onChange={() => {
                      setRemovalCacheChoice("retain");
                      setRemovalConfirmed(false);
                    }}
                  />
                  <span>{t("settings.accounts.removal.retainCache")}</span>
                </label>
                <p className="account-form__hint">
                  {t("settings.accounts.removal.retainCacheHelp")}
                </p>
                <label className="checkbox-row">
                  <input
                    type="checkbox"
                    checked={removalConfirmed}
                    onChange={(event) =>
                      setRemovalConfirmed(event.target.checked)
                    }
                  />
                  <span>{t("settings.accounts.removal.confirm")}</span>
                </label>
              </fieldset>
              <div className="settings-panel__actions">
                <button
                  type="button"
                  onClick={() => void handleLoadAuthStatus()}
                  disabled={!onGetAccountAuthStatus}
                >
                  {t("settings.accounts.actions.status")}
                </button>
                <button type="submit" disabled={!canSubmitReauth()}>
                  {t("settings.accounts.actions.reauth")}
                </button>
                <button
                  type="button"
                  onClick={() => void handleRemoveAccount()}
                  disabled={!onRemoveAccount || !removalConfirmed}
                >
                  {t("settings.accounts.actions.remove")}
                </button>
              </div>
            </form>
          </>
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

            void handleSaveAccountSettings();
          }}
        >
          <label>
            <span>{t("settings.fields.account")}</span>
            <select
              value={activeSelectedAccountId}
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

  async function handleLanguagePreferenceChange(
    language: UiLanguagePreference | null,
  ) {
    if (!onChangeLanguagePreference) {
      setLanguageError(t("settings.language.unavailable"));
      return;
    }

    setLanguageError(null);
    setLanguageNoticeKey(null);
    setLanguageSaving(true);
    try {
      await onChangeLanguagePreference(language);
      setLanguageNoticeKey("settings.language.saved");
    } catch (error) {
      setLanguageError(formatAccountError(error));
    } finally {
      setLanguageSaving(false);
    }
  }

  async function handleExportDiagnostics() {
    if (!onExportDiagnostics) {
      setDiagnosticsError(t("settings.diagnostics.unavailable"));
      return;
    }

    setDiagnosticsNotice(null);
    setDiagnosticsError(null);
    setDiagnosticsExporting(true);
    try {
      const response = await onExportDiagnostics({
        userRequested: true,
        includeSyntheticFixtures,
      });
      setDiagnosticsNotice(
        t("settings.diagnostics.exported", {
          filename: response.destinationLabel,
          count: response.includedLogEntries,
        }),
      );
    } catch (error) {
      setDiagnosticsError(formatAccountError(error));
    } finally {
      setDiagnosticsExporting(false);
    }
  }

  async function handleRefreshDiagnosticsHealth() {
    if (!onLoadDiagnosticsHealth) {
      setDiagnosticsHealthError(t("settings.diagnostics.health.unavailable"));
      return;
    }

    setDiagnosticsHealthError(null);
    setDiagnosticsHealthLoading(true);
    try {
      setDiagnosticsHealth(await onLoadDiagnosticsHealth());
    } catch (error) {
      setDiagnosticsHealthError(formatAccountError(error));
    } finally {
      setDiagnosticsHealthLoading(false);
    }
  }

  function renderDiagnosticsHealth() {
    if (!diagnosticsHealth) {
      return null;
    }

    const attachmentIssueTotal =
      diagnosticsHealth.attachments.missingFiles +
      diagnosticsHealth.attachments.unreadableFiles +
      diagnosticsHealth.attachments.invalidPaths +
      diagnosticsHealth.attachments.oversizedFiles +
      diagnosticsHealth.attachments.sizeMismatches +
      diagnosticsHealth.attachments.hashMismatches;

    return (
      <div className="diagnostics-health__body">
        <p
          className={`diagnostics-health__status diagnostics-health__status--${diagnosticsHealth.status}`}
          role="status"
        >
          {t(`settings.diagnostics.health.status.${diagnosticsHealth.status}`)}
        </p>
        <div className="diagnostics-health__grid">
          <DiagnosticsHealthMetric
            label={t("settings.diagnostics.health.cache")}
            value={t(
              `settings.diagnostics.health.migration.${diagnosticsHealth.cache.migrationStatus}`,
              {
                current:
                  diagnosticsHealth.cache.schemaVersion ??
                  t("settings.diagnostics.health.notAvailable"),
                latest: diagnosticsHealth.cache.latestSchemaVersion,
              },
            )}
          />
          <DiagnosticsHealthMetric
            label={t("settings.diagnostics.health.messages")}
            value={numberFormatter.format(diagnosticsHealth.cache.messageCount)}
          />
          <DiagnosticsHealthMetric
            label={t("settings.diagnostics.health.accounts")}
            value={numberFormatter.format(diagnosticsHealth.cache.accountCount)}
          />
          <DiagnosticsHealthMetric
            label={t("settings.diagnostics.health.retries")}
            value={numberFormatter.format(
              diagnosticsHealth.sync.retryEventCount,
            )}
          />
          <DiagnosticsHealthMetric
            label={t("settings.diagnostics.health.logIssues")}
            value={t("settings.diagnostics.health.warningErrorCount", {
              warnings: numberFormatter.format(
                diagnosticsHealth.sync.warningCount,
              ),
              errors: numberFormatter.format(
                diagnosticsHealth.sync.errorCount,
              ),
            })}
          />
          <DiagnosticsHealthMetric
            label={t("settings.diagnostics.health.attachmentIssues")}
            value={numberFormatter.format(attachmentIssueTotal)}
          />
        </div>

        <section className="diagnostics-health__group">
          <h5>{t("settings.diagnostics.health.categories")}</h5>
          <ul>
            {Object.entries(diagnosticsHealth.sync.categoryCounts).length ===
            0 ? (
              <li>{t("settings.diagnostics.health.noLogCategories")}</li>
            ) : (
              Object.entries(diagnosticsHealth.sync.categoryCounts).map(
                ([category, count]) => (
                  <li key={category}>
                    {t(`settings.diagnostics.health.category.${category}`, {
                      defaultValue: category,
                    })}
                    : {numberFormatter.format(count)}
                  </li>
                ),
              )
            )}
          </ul>
        </section>

        <section className="diagnostics-health__group">
          <h5>{t("settings.diagnostics.health.attachments")}</h5>
          <ul>
            <li>
              {t("settings.diagnostics.health.attachmentStored", {
                stored: numberFormatter.format(
                  diagnosticsHealth.attachments.storedCount,
                ),
                metadata: numberFormatter.format(
                  diagnosticsHealth.attachments.metadataCount,
                ),
              })}
            </li>
            <li>
              {t("settings.diagnostics.health.attachmentChecked", {
                checked: numberFormatter.format(
                  diagnosticsHealth.attachments.checkedFiles,
                ),
              })}
            </li>
            <li>
              {t("settings.diagnostics.health.attachmentIssueBreakdown", {
                missing: numberFormatter.format(
                  diagnosticsHealth.attachments.missingFiles,
                ),
                unreadable: numberFormatter.format(
                  diagnosticsHealth.attachments.unreadableFiles,
                ),
                invalid: numberFormatter.format(
                  diagnosticsHealth.attachments.invalidPaths,
                ),
                size: numberFormatter.format(
                  diagnosticsHealth.attachments.sizeMismatches,
                ),
                hash: numberFormatter.format(
                  diagnosticsHealth.attachments.hashMismatches,
                ),
              })}
            </li>
          </ul>
        </section>

        <section className="diagnostics-health__group">
          <h5>{t("settings.diagnostics.health.remediation")}</h5>
          <ul>
            {diagnosticsHealth.remediationSteps.map((step) => (
              <li key={step}>
                {t(`settings.diagnostics.health.remediationSteps.${step}`)}
              </li>
            ))}
          </ul>
        </section>

        <section className="diagnostics-health__group">
          <h5>{t("settings.diagnostics.health.recentEvents")}</h5>
          <ul>
            {diagnosticsHealth.sync.recentEvents.length === 0 ? (
              <li>{t("settings.diagnostics.health.noRecentEvents")}</li>
            ) : (
              diagnosticsHealth.sync.recentEvents.map((event) => (
                <li key={diagnosticsHealthEventKey(event)}>
                  {formatDiagnosticsEvent(event)}
                </li>
              ))
            )}
          </ul>
        </section>
      </div>
    );
  }

  function formatDiagnosticsEvent(event: DiagnosticsHealthEvent) {
    return t("settings.diagnostics.health.event", {
      time: dateFormatter.format(new Date(event.timestampUnixMs)),
      level: t(`settings.diagnostics.health.level.${event.level}`),
      category: t(
        `settings.diagnostics.health.category.${event.category}`,
        {
          defaultValue: event.category,
        },
      ),
      component: event.component,
      event: event.event,
      state: event.state ?? t("settings.diagnostics.health.notAvailable"),
      action: event.action ?? t("settings.diagnostics.health.notAvailable"),
      code: event.code ?? t("settings.diagnostics.health.notAvailable"),
    });
  }

  function updateAccountForm(update: Partial<AccountFormState>) {
    setAccountForm((current) => ({ ...current, ...update }));
  }

  function handleDetectAccountSettings({ silent }: { silent: boolean }) {
    const detected = inferAccountSettings(accountForm.displayEmail);
    if (!detected) {
      if (!silent) {
        setAccountNotice(t("settings.accounts.notices.detectedFallback"));
      }
      const domain = accountForm.displayEmail.split("@")[1]?.trim().toLowerCase();
      if (domain) {
        setAccountForm((current) => ({
          ...current,
          authKind: "app_password",
          imapHost: current.imapHost || `imap.${domain}`,
          imapPort: current.imapPort || "993",
          imapSecurity: current.imapSecurity || "tls",
          providerKind: "generic_imap_smtp",
          smtpHost: current.smtpHost || `smtp.${domain}`,
          smtpPort: current.smtpPort || "587",
          smtpSecurity: current.smtpSecurity || "starttls",
          username: current.username || current.displayEmail,
        }));
        setAccountSetupStep("connection");
      }
      return;
    }

    setAccountForm((current) => ({
      ...current,
      ...detected,
      displayEmail: current.displayEmail,
      displayName: current.displayName,
      secret: detected.providerKind === "generic_imap_smtp" ? current.secret : "",
      oauthClientId:
        detected.providerKind === "generic_imap_smtp"
          ? ""
          : current.oauthClientId,
    }));
    setAccountSetupStep(
      detected.providerKind === "generic_imap_smtp" ? "connection" : "finish",
    );
    if (!silent) {
      setAccountNotice(
        t("settings.accounts.notices.detected", {
          provider: t(`settings.accounts.providers.${detected.providerKind}`),
        }),
      );
    }
  }

  function buildSetupRequest(): AccountSetupRequest {
    return {
      providerKind: accountForm.providerKind,
      displayEmail: accountForm.displayEmail,
      displayName: accountForm.displayName.trim() || null,
      username: accountForm.username,
      authKind: accountForm.authKind,
      secret: accountForm.secret,
      imapHost: accountForm.imapHost,
      imapPort: Number(accountForm.imapPort) || null,
      imapSecurity: accountForm.imapSecurity,
      smtpHost: accountForm.smtpHost,
      smtpPort: Number(accountForm.smtpPort) || null,
      smtpSecurity: accountForm.smtpSecurity,
    };
  }

  function buildOAuthSetupRequest(): OAuthAccountSetupRequest {
    return {
      providerKind: accountForm.providerKind as OAuthAccountProviderKind,
      displayEmail: accountForm.displayEmail,
      displayName: accountForm.displayName.trim() || null,
      username: accountForm.username,
      clientId: accountForm.oauthClientId,
    };
  }

  function canSubmitAccountSetup() {
    return accountForm.providerKind === "generic_imap_smtp"
      ? Boolean(onCreateAccount)
      : Boolean(onStartOAuthAccountSetup);
  }

  function canSubmitReauth() {
    return selectedAccountUsesOAuth
      ? Boolean(onStartOAuthAccountReauth)
      : Boolean(onReauthAccount);
  }

  function canSaveAccountSettings() {
    return Boolean(selectedAccount && (onSaveAccountSettings || onSaveSignature));
  }

  async function handleSaveAccountSettings() {
    if (!selectedAccount) {
      setAccountError(t("settings.accounts.errors.settingsUnavailable"));
      return;
    }

    const signatureHtml = sanitized.html.trim();
    if (!onSaveAccountSettings) {
      if (!onSaveSignature) {
        setAccountError(t("settings.accounts.errors.settingsUnavailable"));
        return;
      }

      await runAccountAction(async () => {
        await onSaveSignature(selectedAccount.id, signatureHtml || null);
        setDraftsByAccount((current) => ({
          ...current,
          [selectedAccount.id]: signatureHtml,
        }));
        setAccountNotice(t("settings.accounts.notices.settingsSaved"));
      });
      return;
    }

    await runAccountAction(async () => {
      const response = await onSaveAccountSettings({
        accountId: selectedAccount.id,
        displayName: selectedDisplayName.trim() || null,
        signatureHtml,
        localPreferences: selectedPreferences,
      });
      setDisplayNamesByAccount((current) => ({
        ...current,
        [selectedAccount.id]:
          response.account.displayName ?? response.account.displayEmail,
      }));
      setDraftsByAccount((current) => ({
        ...current,
        [selectedAccount.id]: response.signatureHtml ?? "",
      }));
      setPreferencesByAccount((current) => ({
        ...current,
        [selectedAccount.id]: response.localPreferences,
      }));
      setAccountNotice(t("settings.accounts.notices.settingsSaved"));
    });
  }

  async function handleTestAccount() {
    if (!onTestAccountConnection) {
      setAccountError(t("settings.accounts.errors.testUnavailable"));
      return;
    }
    if (accountForm.providerKind !== "generic_imap_smtp") {
      setAccountError(t("settings.accounts.errors.oauthTestUnavailable"));
      return;
    }
    await runAccountAction(async () => {
      const response = await onTestAccountConnection(buildSetupRequest());
      setAccountNotice(
        t(
          response.status === "connection-ready"
            ? "settings.accounts.notices.connectionReady"
            : "settings.accounts.notices.testReady",
        ),
      );
    });
  }

  async function handleCreateAccount() {
    if (accountForm.providerKind !== "generic_imap_smtp") {
      if (!onStartOAuthAccountSetup) {
        setAccountError(t("settings.accounts.errors.oauthSetupUnavailable"));
        return;
      }
      await runAccountAction(async () => {
        const response = await onStartOAuthAccountSetup(buildOAuthSetupRequest());
        setSelectedAccountId(response.account.id);
        setAccountNotice(
          t("settings.accounts.notices.created", {
            email: response.account.displayEmail,
          }),
        );
        setAccountForm(defaultAccountForm);
        onAccountChanged?.();
      });
      return;
    }

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
    if (!selectedAccount) {
      setAccountError(t("settings.accounts.errors.reauthUnavailable"));
      return;
    }

    if (selectedAccountUsesOAuth) {
      if (!onStartOAuthAccountReauth) {
        setAccountError(t("settings.accounts.errors.oauthReauthUnavailable"));
        return;
      }
      await runAccountAction(async () => {
        await onStartOAuthAccountReauth({
          accountId: selectedAccount.id,
          clientId: reauthSecret,
        });
        setReauthSecret("");
        setAccountNotice(t("settings.accounts.notices.reauthed"));
        onAccountChanged?.();
      });
      return;
    }

    if (!onReauthAccount) {
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
      const response = await onRemoveAccount({
        accountId: selectedAccount.id,
        deleteLocalCache: removalCacheChoice === "delete",
      });
      setSelectedAccountId(
        accounts.find((account) => account.id !== selectedAccount.id)?.id ??
          "",
      );
      setRemovalConfirmed(false);
      setAccountNotice(
        t("settings.accounts.notices.removed", {
          cache: response.localCacheDeleted
            ? t("settings.accounts.removal.cacheDeleted")
            : t("settings.accounts.removal.cacheRetained"),
          revocation: t(
            `settings.accounts.removal.revocation.${response.providerRevocation}`,
          ),
        }),
      );
      onAccountChanged?.();
    });
  }

  async function handleLoadAuthStatus() {
    if (!onGetAccountAuthStatus || !selectedAccount) {
      setAccountError(t("settings.accounts.errors.statusUnavailable"));
      return;
    }
    await runAccountAction(async () => {
      const status = await onGetAccountAuthStatus({
        accountId: selectedAccount.id,
      });
      setAuthStatus(status);
      onAccountChanged?.();
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

function inferAccountSettings(
  email: string,
): Partial<AccountFormState> & Pick<AccountFormState, "providerKind"> | null {
  const normalizedEmail = email.trim().toLowerCase();
  const domain = normalizedEmail.split("@")[1];
  if (!domain) {
    return null;
  }

  if (domain === "gmail.com" || domain === "googlemail.com") {
    return {
      authKind: "oauth2",
      providerKind: "gmail",
      username: normalizedEmail,
    };
  }

  if (
    [
      "outlook.com",
      "hotmail.com",
      "live.com",
      "msn.com",
      "office365.com",
    ].includes(domain)
  ) {
    return {
      authKind: "oauth2",
      providerKind: "microsoft",
      username: normalizedEmail,
    };
  }

  const genericPresets: Record<string, Partial<AccountFormState>> = {
    "fastmail.com": {
      imapHost: "imap.fastmail.com",
      imapPort: "993",
      imapSecurity: "tls",
      smtpHost: "smtp.fastmail.com",
      smtpPort: "465",
      smtpSecurity: "tls",
    },
    "icloud.com": {
      imapHost: "imap.mail.me.com",
      imapPort: "993",
      imapSecurity: "tls",
      smtpHost: "smtp.mail.me.com",
      smtpPort: "587",
      smtpSecurity: "starttls",
    },
    "me.com": {
      imapHost: "imap.mail.me.com",
      imapPort: "993",
      imapSecurity: "tls",
      smtpHost: "smtp.mail.me.com",
      smtpPort: "587",
      smtpSecurity: "starttls",
    },
    "yahoo.com": {
      imapHost: "imap.mail.yahoo.com",
      imapPort: "993",
      imapSecurity: "tls",
      smtpHost: "smtp.mail.yahoo.com",
      smtpPort: "465",
      smtpSecurity: "tls",
    },
    "zoho.com": {
      imapHost: "imap.zoho.com",
      imapPort: "993",
      imapSecurity: "tls",
      smtpHost: "smtp.zoho.com",
      smtpPort: "465",
      smtpSecurity: "tls",
    },
    "gmx.com": {
      imapHost: "imap.gmx.com",
      imapPort: "993",
      imapSecurity: "tls",
      smtpHost: "mail.gmx.com",
      smtpPort: "587",
      smtpSecurity: "starttls",
    },
    "gmx.net": {
      imapHost: "imap.gmx.net",
      imapPort: "993",
      imapSecurity: "tls",
      smtpHost: "mail.gmx.net",
      smtpPort: "587",
      smtpSecurity: "starttls",
    },
    "mail.com": {
      imapHost: "imap.mail.com",
      imapPort: "993",
      imapSecurity: "tls",
      smtpHost: "smtp.mail.com",
      smtpPort: "587",
      smtpSecurity: "starttls",
    },
  };

  const preset = genericPresets[domain];
  if (!preset) {
    return null;
  }

  return {
    ...preset,
    authKind: "app_password",
    providerKind: "generic_imap_smtp",
    username: normalizedEmail,
  };
}

function DiagnosticsHealthMetric({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  return (
    <div className="diagnostics-health__metric">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function diagnosticsHealthEventKey(event: DiagnosticsHealthEvent) {
  return [
    event.timestampUnixMs,
    event.level,
    event.component,
    event.event,
    event.state ?? "",
    event.action ?? "",
    event.code ?? "",
  ].join(":");
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
