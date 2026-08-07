import {
  createInstance,
  type i18n as I18nInstance,
  type Resource,
} from "i18next";
import { initReactI18next } from "react-i18next";

import en from "@/locales/en.json";
import zhHans from "@/locales/zh-Hans.json";
import zhHant from "@/locales/zh-Hant.json";

import {
  APP_LANGUAGES,
  DEFAULT_LANGUAGE,
  resolveAppLanguage,
  readLanguagePreference,
  writeLanguagePreference,
  type AppLanguage,
  type AppLanguagePreference,
} from "./language";

export {
  APP_LANGUAGES,
  APP_LANGUAGE_PREFERENCES,
  DEFAULT_LANGUAGE,
  DEFAULT_LANGUAGE_PREFERENCE,
  LANGUAGE_STORAGE_KEY,
  isAppLanguage,
  isAppLanguagePreference,
  readLanguagePreference,
  resolveAppLanguage,
  resolveLanguagePreference,
  resolveSystemAppLanguage,
  writeLanguagePreference,
} from "./language";
export type { AppLanguage, AppLanguagePreference } from "./language";

function getSystemLanguageCandidates(): readonly string[] {
  if (typeof navigator === "undefined") return [];
  return navigator.languages.length > 0
    ? navigator.languages
    : navigator.language
      ? [navigator.language]
      : [];
}

function getLanguageStorage(): Storage | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export function readStoredLanguagePreference(): AppLanguagePreference {
  return readLanguagePreference(getLanguageStorage());
}

export function createAppI18n(
  resources: Resource,
  language: AppLanguage,
): I18nInstance {
  const instance = createInstance();
  void instance.use(initReactI18next).init({
    resources,
    lng: language,
    fallbackLng: DEFAULT_LANGUAGE,
    supportedLngs: APP_LANGUAGES,
    load: "currentOnly",
    returnEmptyString: false,
    initAsync: false,
    interpolation: {
      escapeValue: false,
    },
  });
  return instance;
}

export const i18n = createAppI18n(
  {
    en: { translation: en },
    "zh-Hant": { translation: zhHant },
    "zh-Hans": { translation: zhHans },
  },
  resolveAppLanguage(
    readStoredLanguagePreference(),
    getSystemLanguageCandidates(),
  ),
);

function applyDocumentLanguage(language: AppLanguage) {
  if (typeof document !== "undefined") {
    document.documentElement.lang = language;
  }
}

applyDocumentLanguage(resolveAppLanguage(i18n.resolvedLanguage));

export async function setAppLanguage(
  preference: AppLanguagePreference,
): Promise<void> {
  const language = resolveAppLanguage(
    preference,
    getSystemLanguageCandidates(),
  );
  writeLanguagePreference(getLanguageStorage(), preference);
  await i18n.changeLanguage(language);
  applyDocumentLanguage(language);
}
