export const LANGUAGE_STORAGE_KEY = "buzz.locale";
export const supportedLanguages = ["pt-BR", "en-US"] as const;
export type SupportedLanguage = (typeof supportedLanguages)[number];

type LanguageStorageReader = Pick<Storage, "getItem">;

export function normalizeLanguage(language?: string | null): SupportedLanguage {
  return language?.toLowerCase().startsWith("pt") ? "pt-BR" : "en-US";
}

export function detectInitialLanguage(
  storage?: LanguageStorageReader,
  navigatorLanguage?: string,
): SupportedLanguage {
  const persisted = storage?.getItem(LANGUAGE_STORAGE_KEY);
  if (supportedLanguages.includes(persisted as SupportedLanguage)) {
    return persisted as SupportedLanguage;
  }
  return normalizeLanguage(navigatorLanguage);
}
