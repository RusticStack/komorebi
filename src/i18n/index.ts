import i18next, { type i18n as I18nInstance } from "i18next";
import { initReactI18next } from "react-i18next";
import {
  detectInitialLanguage,
  fallbackLanguage,
  supportedLanguages,
  type SupportedLanguage,
} from "./language";
import { defaultNamespace, resources } from "./resources";

export function configureI18n(
  instance: I18nInstance,
  language: SupportedLanguage = detectInitialLanguage(),
) {
  if (instance.isInitialized) {
    return instance;
  }

  void instance.use(initReactI18next).init({
    defaultNS: defaultNamespace,
    fallbackLng: fallbackLanguage,
    initAsync: false,
    interpolation: {
      escapeValue: false,
    },
    lng: language,
    ns: [defaultNamespace],
    react: {
      useSuspense: false,
    },
    resources,
    supportedLngs: [...supportedLanguages],
  });

  return instance;
}

export function createKomorebiI18n(language: SupportedLanguage = fallbackLanguage) {
  return configureI18n(i18next.createInstance(), language);
}

export const i18n = configureI18n(i18next);

export { detectInitialLanguage, matchSupportedLanguage } from "./language";
export {
  formatByteSize,
  formatCount,
  formatDateTime,
  localeForLanguage,
  useLocaleFormatters,
} from "./localeFormat";
export { resources } from "./resources";
export type { SupportedLanguage } from "./language";
