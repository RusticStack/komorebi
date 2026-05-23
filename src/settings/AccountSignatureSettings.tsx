import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { SafeEmailBody } from "../mail/rendering/SafeEmailBody";
import { sanitizeSignatureHtml } from "../mail/signatures/signatureHtml";

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
  onClose?: () => void;
};

export function AccountSignatureSettings({
  accounts,
  initialAccountId,
  onSaveSignature,
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
  const selectedAccount =
    accounts.find((account) => account.id === selectedAccountId) ?? null;
  const draftHtml = selectedAccount
    ? draftsByAccount[selectedAccount.id] ?? selectedAccount.signatureHtml ?? ""
    : "";
  const sanitized = useMemo(
    () => sanitizeSignatureHtml(draftHtml),
    [draftHtml],
  );

  if (accounts.length === 0) {
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
        <p className="empty-note">{t("mail.states.noAccounts")}</p>
      </section>
    );
  }

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

        <section className="signature-preview" aria-label={t("settings.aria.preview")}>
          <SafeEmailBody bodyHtml={sanitized.html} bodyText={sanitized.plainText} />
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
    </section>
  );
}
