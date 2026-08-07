export const APP_LANGUAGES = ["en", "zh-Hant", "zh-Hans"] as const;
export type AppLanguage = (typeof APP_LANGUAGES)[number];

export const APP_LANGUAGE_PREFERENCES = ["system", ...APP_LANGUAGES] as const;
export type AppLanguagePreference = (typeof APP_LANGUAGE_PREFERENCES)[number];

export const DEFAULT_LANGUAGE: AppLanguage = "en";
export const DEFAULT_LANGUAGE_PREFERENCE: AppLanguagePreference = "system";
export const LANGUAGE_STORAGE_KEY = "buzz-language";

type LanguageStorageReader = Pick<Storage, "getItem">;
type LanguageStorageWriter = Pick<Storage, "setItem">;

export function isAppLanguage(value: unknown): value is AppLanguage {
  return (
    typeof value === "string" &&
    (APP_LANGUAGES as readonly string[]).includes(value)
  );
}

export function isAppLanguagePreference(
  value: unknown,
): value is AppLanguagePreference {
  return value === "system" || isAppLanguage(value);
}

export function resolveLanguagePreference(
  value: unknown,
): AppLanguagePreference {
  return isAppLanguagePreference(value) ? value : DEFAULT_LANGUAGE_PREFERENCE;
}

export function readLanguagePreference(
  storage: LanguageStorageReader | null,
): AppLanguagePreference {
  if (!storage) return DEFAULT_LANGUAGE_PREFERENCE;
  try {
    return resolveLanguagePreference(storage.getItem(LANGUAGE_STORAGE_KEY));
  } catch {
    return DEFAULT_LANGUAGE_PREFERENCE;
  }
}

export function writeLanguagePreference(
  storage: LanguageStorageWriter | null,
  preference: AppLanguagePreference,
): void {
  try {
    storage?.setItem(LANGUAGE_STORAGE_KEY, preference);
  } catch {
    // The language still applies for this session when storage is unavailable.
  }
}

function normalizeLocale(value: string): string {
  return value.trim().replaceAll("_", "-").toLowerCase();
}

function hasRegion(locale: string, region: string): boolean {
  return (
    locale === region ||
    locale.includes(`-${region}-`) ||
    locale.endsWith(`-${region}`)
  );
}

function resolveChineseLocale(locale: string): AppLanguage | null {
  if (!locale.startsWith("zh")) return null;
  if (
    locale.includes("hans") ||
    ["cn", "sg", "my"].some((region) => hasRegion(locale, region))
  ) {
    return "zh-Hans";
  }
  if (
    locale.includes("hant") ||
    ["tw", "hk", "mo"].some((region) => hasRegion(locale, region))
  ) {
    return "zh-Hant";
  }
  return "zh-Hans";
}

export function resolveSystemAppLanguage(
  systemLanguages: readonly unknown[],
): AppLanguage {
  for (const candidate of systemLanguages) {
    if (typeof candidate !== "string") continue;
    const locale = normalizeLocale(candidate);
    const chineseLanguage = resolveChineseLocale(locale);
    if (chineseLanguage) return chineseLanguage;
    if (locale === "en" || locale.startsWith("en-")) return "en";
  }
  return DEFAULT_LANGUAGE;
}

export function resolveAppLanguage(
  preference: unknown,
  systemLanguages: readonly unknown[] = [],
): AppLanguage {
  return isAppLanguage(preference)
    ? preference
    : resolveSystemAppLanguage(systemLanguages);
}
