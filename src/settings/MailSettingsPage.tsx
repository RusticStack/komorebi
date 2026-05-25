import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, Leaf } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "../components/ui/card";
import {
  detectInitialLanguage,
  i18n as komorebiI18n,
} from "../i18n";
import type { MailClientShellProps } from "../mail/shell/MailClientShell";
import {
  accountLocalPreferences,
  emptyMailShellData,
  type MailShellData,
  type UiPreferencesView,
} from "../mail/shell/mailShellState";
import { AccountSignatureSettings } from "./AccountSignatureSettings";

type MailSettingsPageProps = {
  mailClientProps: MailClientShellProps;
  onClose: () => void;
  toolbarAction?: ReactNode;
};

export function MailSettingsPage({
  mailClientProps,
  onClose,
  toolbarAction,
}: MailSettingsPageProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const mailShellQuery = useQuery({
    initialData: mailClientProps.initialData,
    queryFn: () =>
      mailClientProps.loadMailShellData?.() ??
      Promise.resolve(mailClientProps.initialData ?? emptyMailShellData),
    queryKey: ["mail-shell", "settings"],
  });
  const languageQuery = useQuery({
    queryFn: () =>
      mailClientProps.loadUiPreferences?.() ??
      Promise.resolve<UiPreferencesView>({ language: null }),
    queryKey: ["ui-preferences"],
  });
  const languageMutation = useMutation({
    mutationFn: async (language: UiPreferencesView["language"]) => {
      if (!mailClientProps.updateUiPreferences) {
        throw new Error(t("settings.language.unavailable"));
      }

      const preferences = await mailClientProps.updateUiPreferences({
        language,
      });
      await komorebiI18n.changeLanguage(
        preferences.language ?? detectInitialLanguage(),
      );
      return preferences;
    },
    onSuccess: (preferences) => {
      queryClient.setQueryData(["ui-preferences"], preferences);
    },
  });
  const mailShellData = mailShellQuery.data ?? emptyMailShellData;
  const accountViews = accountManagementViews(mailShellData);

  async function invalidateSettingsMailData() {
    await queryClient.invalidateQueries({ queryKey: ["mail-shell", "settings"] });
  }

  return (
    <main className="settings-page">
      <header className="settings-page__titlebar">
        <Button aria-label={t("common.actions.close")} type="button" variant="ghost" onClick={onClose}>
          <ArrowLeft aria-hidden="true" size={16} />
          {t("common.actions.close")}
        </Button>
        <div>
          <p className="eyebrow">{t("settings.section")}</p>
          <h1>{t("settings.aria.panel")}</h1>
        </div>
        {toolbarAction}
      </header>

      <section className="settings-page__grid">
        <Card className="settings-page__intro">
          <CardHeader>
            <span className="settings-page__mark" aria-hidden="true">
              <Leaf size={22} />
            </span>
            <CardTitle>{t("settings.accounts.wizard.heading")}</CardTitle>
          </CardHeader>
          <CardContent>
            <p>{t("settings.accounts.wizard.copy")}</p>
            {mailShellQuery.isError ? (
              <p className="settings-error" role="alert">
                {t("mail.states.accountsError")}
              </p>
            ) : null}
          </CardContent>
        </Card>

        <AccountSignatureSettings
          accounts={accountViews}
          defaultTab="accounts"
          languagePreference={languageQuery.data?.language ?? null}
          languagePreferenceStatus={
            languageQuery.isLoading
              ? "loading"
              : languageQuery.isError
                ? "error"
                : "ready"
          }
          onAccountChanged={invalidateSettingsMailData}
          onChangeLanguagePreference={
            mailClientProps.updateUiPreferences
              ? async (language) => {
                  await languageMutation.mutateAsync(language);
                }
              : undefined
          }
          onClose={onClose}
          onCreateAccount={mailClientProps.createAccount}
          onExportDiagnostics={mailClientProps.exportDiagnosticsBundle}
          onGetAccountAuthStatus={mailClientProps.getAccountAuthStatus}
          onLoadDiagnosticsHealth={mailClientProps.loadDiagnosticsHealth}
          onReauthAccount={mailClientProps.reauthAccount}
          onRemoveAccount={mailClientProps.removeAccount}
          onSaveAccountSettings={mailClientProps.updateAccountSettings}
          onStartOAuthAccountReauth={mailClientProps.startOAuthAccountReauth}
          onStartOAuthAccountSetup={mailClientProps.startOAuthAccountSetup}
          onTestAccountConnection={mailClientProps.testAccountConnection}
        />
      </section>
    </main>
  );
}

function accountManagementViews(data: MailShellData) {
  return data.accounts.map((account) => ({
    authKind: account.authKind,
    displayName: account.displayName ?? null,
    email: account.email,
    id: account.id,
    label: account.label,
    localPreferences: accountLocalPreferences(account),
    providerKind: account.providerKind,
    signatureHtml: account.signatureHtml,
  }));
}
