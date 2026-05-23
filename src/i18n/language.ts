export const supportedLanguages = ["en", "pt-PT"] as const;

export type SupportedLanguage = (typeof supportedLanguages)[number];

export const fallbackLanguage: SupportedLanguage = "en";

export function matchSupportedLanguage(
  languageTag: string | null | undefined,
): SupportedLanguage | null {
  if (!languageTag) {
    return null;
  }

  const normalized = languageTag.replace("_", "-").toLocaleLowerCase();
  if (normalized === "pt" || normalized.startsWith("pt-")) {
    return "pt-PT";
  }
  if (normalized === "en" || normalized.startsWith("en-")) {
    return "en";
  }

  return null;
}

export function detectInitialLanguage(
  navigatorLike: Pick<Navigator, "language" | "languages"> | undefined =
    globalThis.navigator,
): SupportedLanguage {
  const candidates =
    navigatorLike?.languages && navigatorLike.languages.length > 0
      ? navigatorLike.languages
      : [navigatorLike?.language].filter(Boolean);

  for (const candidate of candidates) {
    const supported = matchSupportedLanguage(candidate);
    if (supported) {
      return supported;
    }
  }

  return fallbackLanguage;
}
