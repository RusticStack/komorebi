import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { useLocaleFormatters } from "../../i18n";
import type {
  ComposerDraft,
  LoadStatus,
  MailAccount,
} from "./mailShellState";
import { StatusList } from "./StatusList";

export function DraftsDrawer({
  accounts,
  drafts,
  loadState,
  onClose,
  onOpenDraft,
  onRetry,
}: {
  accounts: MailAccount[];
  drafts: ComposerDraft[];
  loadState: LoadStatus;
  onClose: () => void;
  onOpenDraft: (draftId: string) => void;
  onRetry?: () => void;
}) {
  const { t } = useTranslation();
  const localeFormat = useLocaleFormatters();
  const accountById = useMemo(
    () => new Map(accounts.map((account) => [account.id, account])),
    [accounts],
  );

  return (
    <section className="composer-panel" aria-label={t("composer.drafts.aria")}>
      <header className="settings-panel__header">
        <div>
          <p className="eyebrow">{t("composer.drafts.eyebrow")}</p>
          <h2>{t("composer.drafts.heading")}</h2>
        </div>
        <button type="button" onClick={onClose}>
          {t("common.actions.close")}
        </button>
      </header>

      <section className="draft-list-panel" aria-label={t("composer.drafts.aria")}>
        <StatusList
          status={loadState}
          loadingLabel={t("composer.drafts.loading")}
          errorLabel={t("composer.drafts.error")}
          empty={drafts.length === 0}
          emptyLabel={t("composer.drafts.empty")}
          onRetry={onRetry}
        >
          <ul className="draft-list">
            {drafts.map((draft) => {
              const account = accountById.get(draft.accountId);
              const subject = draftSubjectLabel(draft, t);

              return (
                <li key={draft.id}>
                  <button
                    type="button"
                    onClick={() => onOpenDraft(draft.id)}
                    aria-label={t("composer.drafts.openAria", { subject })}
                  >
                    <strong>{subject}</strong>
                    <span>
                      {account?.label ?? account?.email ?? draft.accountId}
                      {" - "}
                      {draftRecipientPreview(draft, t)}
                    </span>
                    <small>
                      {t("composer.drafts.attachmentCount", {
                        count: draft.attachments.length,
                        formattedCount: localeFormat.count(
                          draft.attachments.length,
                        ),
                      })}
                    </small>
                  </button>
                </li>
              );
            })}
          </ul>
        </StatusList>
      </section>
    </section>
  );
}

function draftSubjectLabel(draft: ComposerDraft, t: TFunction) {
  return draft.subject.trim() || t("composer.drafts.noSubject");
}

function draftRecipientPreview(draft: ComposerDraft, t: TFunction) {
  const recipients = [...draft.to, ...draft.cc, ...draft.bcc];
  if (recipients.length === 0) {
    return t("composer.drafts.noRecipients");
  }

  const [firstRecipient, ...remainingRecipients] = recipients;
  const label = firstRecipient
    ? formatDraftRecipient(firstRecipient)
    : t("composer.drafts.noRecipients");
  if (remainingRecipients.length === 0) {
    return label;
  }

  return t("composer.drafts.recipientSummary", {
    count: remainingRecipients.length,
    first: label,
    formattedCount: remainingRecipients.length,
  });
}

function formatDraftRecipient(address: { name: string | null; address: string }) {
  return address.name ? `${address.name} <${address.address}>` : address.address;
}
