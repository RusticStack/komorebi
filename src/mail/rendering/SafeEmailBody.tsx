import { useMemo, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";
import {
  sanitizeEmailHtml,
  type SafeEmailLink,
  type SanitizedEmailHtml,
} from "./emailHtml";

export type SafeEmailBodyProps = {
  bodyHtml?: string | null;
  bodyText?: string | null;
  onOpenExternalLink?: (link: SafeEmailLink) => void;
};

export function SafeEmailBody({
  bodyHtml,
  bodyText,
  onOpenExternalLink,
}: SafeEmailBodyProps) {
  const { t } = useTranslation();
  const sanitized = useMemo(
    () => (bodyHtml ? sanitizeEmailHtml(bodyHtml) : null),
    [bodyHtml],
  );
  const hasRenderableHtml = Boolean(sanitized?.html.trim());
  const fallbackText = bodyText?.trim() || sanitized?.plainText || "";

  function handleBodyClick(event: MouseEvent<HTMLDivElement>) {
    const target = event.target;

    if (!(target instanceof Element)) {
      return;
    }

    const anchor = target.closest<HTMLAnchorElement>("a[data-komorebi-url]");

    if (!anchor) {
      return;
    }

    event.preventDefault();

    const link = findLink(sanitized, anchor.dataset.komorebiLinkId);

    if (link) {
      onOpenExternalLink?.(link);
    }
  }

  return (
    <div className="safe-email-body">
      {hasRenderableHtml && sanitized ? (
        <div
          className="safe-email-body__html"
          onClick={handleBodyClick}
          dangerouslySetInnerHTML={{ __html: sanitized.html }}
        />
      ) : (
        <PlainTextEmailBody text={fallbackText} />
      )}

      {sanitized?.remoteContentBlocked ? (
        <p className="safe-email-body__notice" role="status">
          {t("safeEmail.remoteContentBlocked")}
        </p>
      ) : null}
    </div>
  );
}

function PlainTextEmailBody({ text }: { text: string }) {
  const { t } = useTranslation();

  if (!text) {
    return <p className="safe-email-body__empty">{t("safeEmail.emptyBody")}</p>;
  }

  return <pre className="safe-email-body__plain">{text}</pre>;
}

function findLink(
  sanitized: SanitizedEmailHtml | null,
  linkId: string | undefined,
) {
  if (!sanitized || !linkId) {
    return null;
  }

  return sanitized.links.find((link) => link.id === linkId) ?? null;
}
