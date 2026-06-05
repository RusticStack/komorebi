import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { useLocaleFormatters } from "../../i18n";
import type {
  MailSearchParsedFilter,
  MailSearchParsedQuery,
} from "./mailShellState";

export type MailSearchUiStatus =
  | "idle"
  | "debouncing"
  | "loading"
  | "ready"
  | "error"
  | "unavailable";

export function SearchStatusLine({
  isBackendSearchActive,
  resultCount,
  status,
}: {
  isBackendSearchActive: boolean;
  resultCount: number;
  status: MailSearchUiStatus;
}) {
  const { t } = useTranslation();
  const localeFormat = useLocaleFormatters();

  if (!isBackendSearchActive) {
    return null;
  }

  if (status === "debouncing" || status === "loading") {
    return (
      <span className="search-status" role="status">
        {t("mail.search.status.loading")}
      </span>
    );
  }

  if (status === "unavailable") {
    return (
      <span className="search-status search-status--warning" role="status">
        {t("mail.search.status.unavailable")}
      </span>
    );
  }

  if (status === "error") {
    return (
      <span className="search-status search-status--warning" role="status">
        {t("mail.search.error.heading")}
      </span>
    );
  }

  if (status === "ready") {
    return (
      <span className="search-status" role="status">
        {localeFormat.searchResults(resultCount)}
      </span>
    );
  }

  return null;
}

export function SearchQueryTools({
  currentQuery,
  parsedQuery,
  recentSearches,
  savedSearches,
  onApplySearch,
  onClearHistory,
  onRemoveSavedSearch,
  onSaveSearch,
}: {
  currentQuery: string;
  parsedQuery: MailSearchParsedQuery | null;
  recentSearches: string[];
  savedSearches: string[];
  onApplySearch: (query: string) => void;
  onClearHistory: () => void;
  onRemoveSavedSearch: (query: string) => void;
  onSaveSearch: () => void;
}) {
  const { t } = useTranslation();
  const filters = parsedQuery?.filters ?? [];
  const currentQuerySaved = savedSearches.includes(currentQuery);
  const visibleRecentSearches = recentSearches.filter(
    (query) => query !== currentQuery && !savedSearches.includes(query),
  );
  const hasTools =
    filters.length > 0 ||
    Boolean(currentQuery) ||
    savedSearches.length > 0 ||
    visibleRecentSearches.length > 0;

  if (!hasTools) {
    return null;
  }

  return (
    <section className="search-tools" aria-label={t("mail.search.toolsAria")}>
      {filters.length > 0 ? (
        <div
          className="search-filter-chips"
          aria-label={t("mail.search.filtersAria")}
        >
          {filters.map((filter) => (
            <span className="search-chip" key={`${filter.kind}:${filter.value}`}>
              {searchFilterLabel(filter, t)}
            </span>
          ))}
        </div>
      ) : null}
      {currentQuery ? (
        <div className="search-tool-row">
          {currentQuerySaved ? (
            <button
              type="button"
              className="search-tool-button"
              onClick={() => onRemoveSavedSearch(currentQuery)}
            >
              {t("mail.search.saved.removeCurrent")}
            </button>
          ) : (
            <button
              type="button"
              className="search-tool-button"
              onClick={onSaveSearch}
            >
              {t("mail.search.saved.saveCurrent")}
            </button>
          )}
        </div>
      ) : null}
      {savedSearches.length > 0 ? (
        <SearchStoredQueryRow
          label={t("mail.search.saved.label")}
          queries={savedSearches}
          removeLabel={t("mail.search.saved.remove")}
          onApplySearch={onApplySearch}
          onRemoveSearch={onRemoveSavedSearch}
        />
      ) : null}
      {visibleRecentSearches.length > 0 ? (
        <SearchStoredQueryRow
          label={t("mail.search.recent.label")}
          queries={visibleRecentSearches}
          trailingAction={
            <button
              type="button"
              className="search-tool-button"
              onClick={onClearHistory}
            >
              {t("mail.search.recent.clear")}
            </button>
          }
          onApplySearch={onApplySearch}
        />
      ) : null}
    </section>
  );
}

function SearchStoredQueryRow({
  label,
  queries,
  removeLabel,
  trailingAction,
  onApplySearch,
  onRemoveSearch,
}: {
  label: string;
  queries: string[];
  removeLabel?: string;
  trailingAction?: ReactNode;
  onApplySearch: (query: string) => void;
  onRemoveSearch?: (query: string) => void;
}) {
  return (
    <div className="search-tool-row">
      <span>{label}</span>
      <div className="search-token-list">
        {queries.map((query) => (
          <span className="search-token" key={query}>
            <button type="button" onClick={() => onApplySearch(query)}>
              {query}
            </button>
            {onRemoveSearch && removeLabel ? (
              <button
                type="button"
                aria-label={`${removeLabel}: ${query}`}
                onClick={() => onRemoveSearch(query)}
              >
                x
              </button>
            ) : null}
          </span>
        ))}
        {trailingAction}
      </div>
    </div>
  );
}

function searchFilterLabel(filter: MailSearchParsedFilter, t: TFunction) {
  switch (filter.kind) {
    case "from":
      return t("mail.search.filters.from", { value: filter.value });
    case "subject":
      return t("mail.search.filters.subject", { value: filter.value });
    case "body":
      return t("mail.search.filters.body", { value: filter.value });
    case "has":
      return t("mail.search.filters.hasAttachment");
    case "is":
      return t(`mail.search.filters.is.${filter.value}`);
    default:
      return filter.value;
  }
}

export function HighlightedText({
  terms,
  text,
}: {
  terms: string[];
  text: string;
}) {
  const normalizedTerms = normalizeHighlightTerms(terms);
  if (!text || normalizedTerms.length === 0) {
    return <>{text}</>;
  }

  const matcher = new RegExp(
    `(${normalizedTerms.map(escapeRegExp).join("|")})`,
    "gi",
  );
  const parts = text.split(matcher);

  return (
    <>
      {parts.map((part, index) =>
        normalizedTerms.some(
          (term) => part.toLocaleLowerCase() === term.toLocaleLowerCase(),
        ) ? (
          <mark className="search-highlight" key={`${part}-${index}`}>
            {part}
          </mark>
        ) : (
          <span key={`${part}-${index}`}>{part}</span>
        ),
      )}
    </>
  );
}

function normalizeHighlightTerms(terms: string[]) {
  const normalized = new Set<string>();
  for (const term of terms) {
    const value = term.trim().toLocaleLowerCase();
    if (value) {
      normalized.add(value);
    }
  }

  return Array.from(normalized).sort((left, right) => right.length - left.length);
}

function escapeRegExp(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
