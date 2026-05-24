import { useMemo } from "react";
import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import {
  fallbackLanguage,
  matchSupportedLanguage,
  type SupportedLanguage,
} from "./language";

type ByteUnitKey = "byte" | "kilobyte" | "megabyte" | "gigabyte";

const millisecondsPerDay = 24 * 60 * 60 * 1000;

const byteUnits: Array<{ divisor: number; key: ByteUnitKey }> = [
  { divisor: 1, key: "byte" },
  { divisor: 1024, key: "kilobyte" },
  { divisor: 1024 ** 2, key: "megabyte" },
  { divisor: 1024 ** 3, key: "gigabyte" },
];

export function localeForLanguage(
  language: string | null | undefined,
): SupportedLanguage {
  return matchSupportedLanguage(language) ?? fallbackLanguage;
}

export function formatCount(
  value: number,
  language: string | null | undefined,
) {
  const count = Number.isFinite(value) ? Math.max(0, Math.trunc(value)) : 0;
  return new Intl.NumberFormat(localeForLanguage(language), {
    maximumFractionDigits: 0,
  }).format(count);
}

export function formatDateTime(
  value: string | null | undefined,
  language: string | null | undefined,
) {
  const trimmed = value?.trim();
  if (!trimmed) {
    return "";
  }

  const date = new Date(trimmed);
  if (Number.isNaN(date.getTime())) {
    return trimmed;
  }

  return new Intl.DateTimeFormat(localeForLanguage(language), {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}

export function formatRelativeDateTime(
  value: string | null | undefined,
  language: string | null | undefined,
  t: TFunction,
  referenceDate = new Date(),
) {
  const trimmed = value?.trim();
  if (!trimmed) {
    return "";
  }

  const date = new Date(trimmed);
  if (Number.isNaN(date.getTime())) {
    return trimmed;
  }

  const locale = localeForLanguage(language);
  const dayDelta = Math.round(
    (startOfLocalDay(date) - startOfLocalDay(referenceDate)) /
      millisecondsPerDay,
  );
  const time = new Intl.DateTimeFormat(locale, {
    timeStyle: "short",
  }).format(date);

  if (dayDelta === 0) {
    return t("common.relativeDate.todayWithTime", { time });
  }
  if (dayDelta === -1) {
    return t("common.relativeDate.yesterdayWithTime", { time });
  }
  if (dayDelta === 1) {
    return t("common.relativeDate.tomorrowWithTime", { time });
  }

  return formatDateTime(trimmed, locale);
}

export function formatByteSize(
  sizeBytes: number,
  language: string | null | undefined,
  t: TFunction,
) {
  const bytes = Number.isFinite(sizeBytes) ? Math.max(0, sizeBytes) : 0;
  let unit = byteUnits[0];
  for (const candidate of byteUnits) {
    if (bytes >= candidate.divisor) {
      unit = candidate;
    }
  }
  const value = unit.key === "byte" ? bytes : bytes / unit.divisor;
  const maximumFractionDigits =
    unit.key === "byte" || value >= 10 ? 0 : 1;
  const formattedValue = new Intl.NumberFormat(localeForLanguage(language), {
    maximumFractionDigits,
  }).format(value);

  return `${formattedValue} ${t(`common.units.bytes.${unit.key}`)}`;
}

export function useLocaleFormatters() {
  const { i18n, t } = useTranslation();
  const language = localeForLanguage(i18n.resolvedLanguage ?? i18n.language);

  return useMemo(
    () => ({
      byteSize(sizeBytes: number) {
        return formatByteSize(sizeBytes, language, t);
      },
      count(value: number) {
        return formatCount(value, language);
      },
      dateTime(value: string | null | undefined) {
        return formatDateTime(value, language);
      },
      relativeDateTime(value: string | null | undefined) {
        return formatRelativeDateTime(value, language, t);
      },
      searchResults(count: number) {
        return t("mail.search.status.result", {
          count,
          formattedCount: formatCount(count, language),
        });
      },
      unreadCount(count: number) {
        return t("mail.unreadCount", {
          count,
          formattedCount: formatCount(count, language),
        });
      },
    }),
    [language, t],
  );
}

function startOfLocalDay(date: Date) {
  return new Date(
    date.getFullYear(),
    date.getMonth(),
    date.getDate(),
  ).getTime();
}
