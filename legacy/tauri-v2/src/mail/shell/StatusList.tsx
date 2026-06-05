import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { MailShellLoadState } from "./mailShellState";

export function StatusList({
  status,
  loadingLabel,
  errorLabel,
  empty,
  emptyLabel,
  children,
  onRetry,
}: {
  status: MailShellLoadState[keyof MailShellLoadState];
  loadingLabel: string;
  errorLabel: string;
  empty: boolean;
  emptyLabel: string;
  children: ReactNode;
  onRetry?: () => void;
}) {
  const { t } = useTranslation();

  if (status === "loading") {
    return <p className="empty-note status-note">{loadingLabel}</p>;
  }

  if (status === "error") {
    return (
      <div className="status-note status-note--error" role="status">
        <p>{errorLabel}</p>
        <button type="button" onClick={onRetry} disabled={!onRetry}>
          {t("common.actions.retry")}
        </button>
      </div>
    );
  }

  if (empty) {
    return <p className="empty-note">{emptyLabel}</p>;
  }

  return children;
}
