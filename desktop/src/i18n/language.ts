/**
 * Device-level language preference for the desktop UI.
 *
 * Model (spec BUZZ-DESKTOP-I18N-ZH-HANS-001 §4.1):
 *
 * ```
 * user preference: system | en | zh-Hans   (persisted under LANGUAGE_STORAGE_KEY)
 * system sources:  navigator.languages, then navigator.language
 * resolution:      en | zh-Hans
 * fallback:        en
 * ```
 *
 * This module is pure UI-agnostic state: it never imports i18next. The i18n
 * singleton (`./index.ts`) subscribes to it, so runtime switches reach both
 * `<html lang>` and the translation catalogs from one code path.
 *
 * Language is a device-level preference, not a community-scoped one: it is
 * stored once per profile, survives community/identity switches, and is reset
 * only by the user choosing a different value.
 */

import * as React from "react";

export type AppLanguage = "en" | "zh-Hans";

/** What the user can pick in Settings → Appearance. */
export type LanguagePreference = "system" | AppLanguage;

/** localStorage key holding the explicit user choice (spec §4.1). */
export const LANGUAGE_STORAGE_KEY = "buzz-language";

/** The one language every missing or empty translation falls back to. */
export const FALLBACK_LANGUAGE: AppLanguage = "en";

export const LANGUAGE_PREFERENCES: readonly LanguagePreference[] = [
  "system",
  "en",
  "zh-Hans",
];

const DEFAULT_PREFERENCE: LanguagePreference = "system";

/**
 * Parse a stored preference. Invalid, corrupted, or unreadable values
 * degrade to "system" rather than to a pinned language (spec T-002) — an
 * unreadable store must never silently pin the user to one language.
 */
export function parseLanguagePreference(
  value: string | null | undefined,
): LanguagePreference {
  return value === "system" || value === "en" || value === "zh-Hans"
    ? value
    : DEFAULT_PREFERENCE;
}

/**
 * Map one system locale candidate to a shipped catalog.
 *
 * - `zh-Hans`, `zh-CN`, `zh-SG`, `zh-MY`, bare `zh`, and underscore/case
 *   variants resolve to `zh-Hans`.
 * - Traditional Chinese (`zh-TW`, `zh-HK`, `zh-MO`, `zh-Hant`) resolves to
 *   `en` while no `zh-Hant` catalog ships — a simplified interface must
 *   never be shown silently to traditional-locale users.
 * - `en` and every other language resolve to `en`.
 */
export function mapSystemLocale(candidate: string): AppLanguage {
  const tag = candidate.toLowerCase().replace(/_/g, "-");
  if (tag === "zh") return "zh-Hans";
  const [language, subtag] = tag.split("-");
  if (language === "zh") {
    const sub = subtag?.toLowerCase();
    if (sub === "cn" || sub === "sg" || sub === "my" || sub === "hans") {
      return "zh-Hans";
    }
    return FALLBACK_LANGUAGE;
  }
  return FALLBACK_LANGUAGE;
}

/**
 * Walk a system language list in preference order and return the first
 * candidate that maps to a non-fallback catalog, else the fallback.
 * `null` / empty lists (no usable system signal) resolve to the fallback.
 */
export function resolveSystemLanguage(
  languages?: readonly string[] | null,
): AppLanguage {
  if (!languages) return FALLBACK_LANGUAGE;
  for (const candidate of languages) {
    if (typeof candidate !== "string" || candidate.length === 0) continue;
    const mapped = mapSystemLocale(candidate);
    if (mapped !== FALLBACK_LANGUAGE) return mapped;
  }
  return FALLBACK_LANGUAGE;
}

/** Combine the user preference with system sources into the live language. */
export function resolveEffectiveLanguage(
  preference: LanguagePreference,
  systemLanguages?: readonly string[] | null,
): AppLanguage {
  if (preference === "system") {
    return resolveSystemLanguage(systemLanguages);
  }
  return preference;
}

const listeners = new Set<() => void>();
let languagePreference: LanguagePreference = DEFAULT_PREFERENCE;
let effectiveLanguage: AppLanguage = FALLBACK_LANGUAGE;
let listeningForStorageChanges = false;

function readSystemLanguages(): readonly string[] | null {
  const nav =
    typeof globalThis.navigator !== "undefined" ? globalThis.navigator : null;
  return nav && Array.isArray(nav.languages) ? nav.languages : null;
}

function readStoredPreference(): LanguagePreference {
  try {
    return parseLanguagePreference(
      globalThis.localStorage?.getItem(LANGUAGE_STORAGE_KEY),
    );
  } catch {
    // Denied/quota-broken storage must never blank or pin the app (NFR-003).
    return DEFAULT_PREFERENCE;
  }
}

function applyEffectiveLanguage(next: AppLanguage): void {
  effectiveLanguage = next;
  // Keep <html lang> in sync on every effective-language change (NFR-001):
  // screen readers and IME see the right language hint immediately.
  globalThis.document?.documentElement?.setAttribute("lang", next);
}

function notifyListeners(): void {
  for (const listener of listeners) listener();
}

/** Re-resolve the preference from storage and system signals. */
function applyStoredPreference(): void {
  const preference = readStoredPreference();
  const next = resolveEffectiveLanguage(preference, readSystemLanguages());
  const changed = next !== effectiveLanguage;
  languagePreference = preference;
  applyEffectiveLanguage(next);
  if (changed) notifyListeners();
}

function listenForStorageChanges(): void {
  if (listeningForStorageChanges || !globalThis.window?.addEventListener) {
    return;
  }
  globalThis.window.addEventListener("storage", (event) => {
    if (event.key === LANGUAGE_STORAGE_KEY || event.key === null) {
      applyStoredPreference();
    }
  });
  listeningForStorageChanges = true;
}

/**
 * Sync the stored preference before React renders so the first paint is
 * already in the right language (no key flash, no visible switch).
 * Idempotent: safe to call on every boot; listeners attach once.
 */
export function initializeLanguagePreference(): void {
  applyStoredPreference();
  listenForStorageChanges();
}

export function getLanguagePreference(): LanguagePreference {
  return languagePreference;
}

export function getEffectiveLanguage(): AppLanguage {
  return effectiveLanguage;
}

/**
 * Apply a new preference immediately. Persistence is best-effort: when
 * localStorage is disabled or over quota the live switch still applies for
 * this session and no error escapes (spec NFR-003).
 */
export function setLanguagePreference(preference: LanguagePreference): void {
  languagePreference = preference;
  applyEffectiveLanguage(
    resolveEffectiveLanguage(preference, readSystemLanguages()),
  );
  try {
    globalThis.localStorage?.setItem(LANGUAGE_STORAGE_KEY, preference);
  } catch {
    // See above: persistence failure is non-fatal.
  }
  notifyListeners();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Raw change subscription — used by the i18n singleton, not by UI. */
export function subscribeLanguage(listener: () => void): () => void {
  return subscribe(listener);
}

export function useLanguagePreference(): LanguagePreference {
  return React.useSyncExternalStore(
    subscribe,
    getLanguagePreference,
    () => DEFAULT_PREFERENCE,
  );
}

export function useEffectiveLanguage(): AppLanguage {
  return React.useSyncExternalStore(
    subscribe,
    getEffectiveLanguage,
    () => FALLBACK_LANGUAGE,
  );
}
