import en from "./locales/en.json";
import ptPT from "./locales/pt-PT.json";

export const defaultNamespace = "translation";

export const resources = {
  en: {
    [defaultNamespace]: en,
  },
  "pt-PT": {
    [defaultNamespace]: ptPT,
  },
} as const;

export { en, ptPT };
