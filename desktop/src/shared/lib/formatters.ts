/**
 * Locale-aware `Intl` formatter factory (spec BUZZ-DESKTOP-I18N-ZH-HANS-001
 * §4.2, FR-006 / AC-006).
 *
 * The problem this replaces: modules built their `Intl.*` formatters at
 * *module load*, hard-coded to `en-US`. A formatter is a frozen snapshot of a
 * locale, so a load-time builder can never follow a runtime language switch —
 * and the switch is the whole point of FR-001. Here the locale is resolved
 * from i18n at **call** time and formatters are cached per
 * `(locale, options)` pair, so the once-per-locale cost stays amortized.
 *
 * English must stay byte-exact (NFR-008), so `en` maps to the same `en-US`
 * tag the old module-level formatters used. Only the *lookup* moved; the
 * options handed to `Intl` did not.
 *
 * Two boundaries this module guards:
 * - Display text is translatable, date *logic* is not. Day boundaries, sort
 *   keys, ISO stamps and cache keys stay locale-free (see `datetime.ts`).
 * - `en` is the fallback for a word as well as for a language: before i18n
 *   initializes — or if a key ever goes missing — callers still get the
 *   English label rather than a raw `common.date.today`.
 */

import { i18n } from "@/i18n";
import { FALLBACK_LANGUAGE, type AppLanguage } from "@/i18n/language";

/** Catalog keys for the date/time display words (spec §4.4: full units). */
export const DATE_WORD_KEYS = {
  today: "common.date.today",
  yesterday: "common.date.yesterday",
  dayAndTime: "common.date.day-and-time",
} as const;

/**
 * English words, identical to the strings these keys replaced. Used only when
 * i18n cannot answer — uninitialized, or a key missing from both catalogs —
 * which is why the `en` catalog repeats them as the real source of truth.
 */
export const ENGLISH_DATE_WORDS: Readonly<
  Record<keyof typeof DATE_WORD_KEYS, string>
> = {
  today: "Today",
  yesterday: "Yesterday",
  dayAndTime: "{{date}} at {{time}}",
};

/** The `en` catalog's language tag, pinned for formatting (NFR-008). */
const FALLBACK_INTL_TAG = "en-US";

/**
 * BCP 47 tags each shipped language formats with. `zh-Hans` is a valid
 * script-only tag, but not every ICU build carries its data, so each tag is
 * verified before use (see `intlTagForLanguage`).
 */
const INTL_TAG_FOR_LANGUAGE: Record<AppLanguage, string> = {
  en: FALLBACK_INTL_TAG,
  "zh-Hans": "zh-Hans",
};

/** Regional fallback for a script-only tag an older ICU build rejects. */
const INTL_TAG_ALTERNATIVES: Record<AppLanguage, string[]> = {
  en: [],
  "zh-Hans": ["zh-CN"],
};

/** Language family each tag must resolve inside to count as usable. */
const INTL_TAG_FAMILY: Record<AppLanguage, string> = {
  en: "en",
  "zh-Hans": "zh",
};

/** Cache of verified tags, so the probe formatter is built at most once per language. */
const verifiedTags = new Map<AppLanguage, string>();

function tagIsUsable(language: AppLanguage, tag: string): boolean {
  try {
    const { locale } = new Intl.DateTimeFormat(tag).resolvedOptions();
    return locale.toLowerCase().startsWith(INTL_TAG_FAMILY[language]);
  } catch {
    // RangeError: malformed tag, or nothing behind it.
    return false;
  }
}

/**
 * Resolve one app language to an `Intl` tag this runtime actually honors.
 * Falls back to the requested tag when ICU has nothing better: a locale the
 * runtime cannot resolve degrades to its own default either way, and pinning
 * the tag keeps the choice in one place.
 */
export function intlTagForLanguage(language: AppLanguage): string {
  const cached = verifiedTags.get(language);
  if (cached) return cached;
  const candidates = [
    INTL_TAG_FOR_LANGUAGE[language],
    ...INTL_TAG_ALTERNATIVES[language],
  ];
  const resolved =
    candidates.find((tag) => tagIsUsable(language, tag)) ?? candidates[0];
  verifiedTags.set(language, resolved);
  return resolved;
}

/** Narrow whatever i18n reports to a language the app ships. */
export function appLanguage(language?: string): AppLanguage {
  return language === "zh-Hans" ? "zh-Hans" : FALLBACK_LANGUAGE;
}

/**
 * The `Intl` tag for the language i18n is showing right now. Read at call
 * time, never captured: a formatter built during module load would outlive
 * every later switch.
 */
export function appIntlLocale(): string {
  return intlTagForLanguage(appLanguage(i18n.language));
}

/**
 * Stable cache key for an options object. Presets are module-level frozen
 * literals, so key order is fixed; sorting anyway keeps the cache correct if
 * a caller ever builds options inline.
 */
function optionsKey(options: Intl.DateTimeFormatOptions): string {
  return Object.keys(options)
    .sort()
    .map(
      (key) =>
        `${key}=${String(options[key as keyof Intl.DateTimeFormatOptions])}`,
    )
    .join("&");
}

const dateTimeCache = new Map<string, Intl.DateTimeFormat>();

/**
 * Cached `Intl.DateTimeFormat` for one `(locale, options)` pair — the
 * replacement for the old module-level `en-US` constants. One instance per
 * distinct pair, so a language switch costs one formatter construction per
 * preset, not one per rendered timestamp.
 */
export function dateTimeFormatter(
  options: Intl.DateTimeFormatOptions,
  locale: string = appIntlLocale(),
): Intl.DateTimeFormat {
  const key = `${locale}\u0000${optionsKey(options)}`;
  const cached = dateTimeCache.get(key);
  if (cached) return cached;
  const built = new Intl.DateTimeFormat(locale, options);
  dateTimeCache.set(key, built);
  return built;
}

/** Distinct formatters built so far — a leak canary for the cache. */
export function formatterCacheSize(): number {
  return dateTimeCache.size;
}

function catalogWord(key: string, english: string): string {
  let value: unknown;
  try {
    value = i18n.t(key);
  } catch {
    value = undefined;
  }
  // Uninitialized i18next answers `undefined`; a missing key answers the key.
  return typeof value === "string" && value.length > 0 && value !== key
    ? value
    : english;
}

/** Display words a date label is made of, resolved in the current language. */
export function dateWords(): {
  today: string;
  yesterday: string;
} {
  return {
    today: catalogWord(DATE_WORD_KEYS.today, ENGLISH_DATE_WORDS.today),
    yesterday: catalogWord(
      DATE_WORD_KEYS.yesterday,
      ENGLISH_DATE_WORDS.yesterday,
    ),
  };
}

/**
 * Fill a `{{name}}` template fetched from the catalog.
 *
 * i18next interpolates the same way (`escapeValue: false`, spec §4.3), but
 * interpolating here keeps the English fallback path — taken when i18n is not
 * initialized yet — byte-identical to the catalog path.
 */
export function fillTemplate(
  template: string,
  variables: Readonly<Record<string, string>>,
): string {
  return template.replace(/\{\{(\w+)\}\}/g, (match, name: string) =>
    name in variables ? variables[name] : match,
  );
}

/**
 * Join a day label and a clock time as one translatable unit — "Yesterday at
 * 2:34 PM", "昨天 14:34". The whole phrase, joiner included, is the catalog
 * value, because the joining word is not universal (spec §4.4).
 */
export function joinDayAndTime(dateLabel: string, time: string): string {
  return fillTemplate(
    catalogWord(DATE_WORD_KEYS.dayAndTime, ENGLISH_DATE_WORDS.dayAndTime),
    { date: dateLabel, time },
  );
}
