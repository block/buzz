import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import { enUS } from "./resources/en-US";
import { ptBR } from "./resources/pt-BR";
import {
  getGlobalAgentConfig,
  setGlobalAgentConfig,
} from "@/shared/api/tauriGlobalAgentConfig";
import { invokeTauri } from "@/shared/api/tauri";
import {
  detectInitialLanguage,
  LANGUAGE_STORAGE_KEY,
  normalizeLanguage,
  type SupportedLanguage,
} from "./language";

type LanguageStorage = Pick<Storage, "getItem" | "setItem">;

const browserStorage =
  typeof window === "undefined" ? undefined : window.localStorage;
const initialLanguage = detectInitialLanguage(
  browserStorage,
  typeof navigator === "undefined" ? undefined : navigator.language,
);

void i18n.use(initReactI18next).init({
  fallbackLng: "en-US",
  interpolation: { escapeValue: false },
  lng: initialLanguage,
  resources: {
    "en-US": { translation: enUS },
    "pt-BR": { translation: ptBR },
  },
});

export async function setLanguage(
  language: SupportedLanguage,
  storage: LanguageStorage | undefined = browserStorage,
): Promise<void> {
  storage?.setItem(LANGUAGE_STORAGE_KEY, language);
  await i18n.changeLanguage(language);
  if (typeof document !== "undefined") {
    document.documentElement.lang = language;
  }
  await syncAgentResponseLanguage(language);
}

export async function syncAgentResponseLanguage(
  language: SupportedLanguage,
): Promise<void> {
  try {
    await invokeTauri("set_speech_language", { language });
  } catch (error) {
    console.warn("Could not synchronize the speech language", error);
  }
  try {
    const config = await getGlobalAgentConfig();
    if (config.env_vars.BUZZ_RESPONSE_LANGUAGE === language) return;
    await setGlobalAgentConfig({
      ...config,
      env_vars: {
        ...config.env_vars,
        BUZZ_RESPONSE_LANGUAGE: language,
      },
    });
  } catch (error) {
    console.warn("Could not synchronize the agent response language", error);
  }
}

export function currentLanguage(): SupportedLanguage {
  return normalizeLanguage(i18n.resolvedLanguage ?? i18n.language);
}

export function formatDate(
  value: Date | number | string,
  options?: Intl.DateTimeFormatOptions,
): string {
  return new Intl.DateTimeFormat(currentLanguage(), options).format(
    new Date(value),
  );
}

export function formatNumber(
  value: number,
  options?: Intl.NumberFormatOptions,
): string {
  return new Intl.NumberFormat(currentLanguage(), options).format(value);
}

if (typeof document !== "undefined") {
  document.documentElement.lang = initialLanguage;
}

export { i18n };
export {
  detectInitialLanguage,
  LANGUAGE_STORAGE_KEY,
  normalizeLanguage,
  supportedLanguages,
  type SupportedLanguage,
} from "./language";
