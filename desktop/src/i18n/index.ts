/**
 * i18n singleton for the desktop app (spec BUZZ-DESKTOP-I18N-ZH-HANS-001 §4.3).
 *
 * - English is the complete source catalog and the final fallback;
 *   Simplified Chinese (`zh-Hans`) is the first added locale.
 * - Catalogs are bundled at build time (no network load, NFR-006): the
 *   production build makes zero new runtime requests for strings.
 * - Init is synchronous (`initImmediate: false` with in-memory resources),
 *   so the first React paint is already in the resolved language.
 * - Translations are data, never HTML: `interpolation.escapeValue` stays
 *   false because React escapes, and no value may carry markup.
 */

import i18next from "i18next";
import { initReactI18next } from "react-i18next";
import en from "@/locales/en.json";
import zhHans from "@/locales/zh-Hans.json";
import {
  FALLBACK_LANGUAGE,
  getEffectiveLanguage,
  initializeLanguagePreference,
  subscribeLanguage,
} from "./language";
import type { AppLanguage } from "./language";

/** The app-wide i18next instance (the `i18next` default singleton). */
export const i18n = i18next;

/** Rust's native-menu command; it accepts exactly the `AppLanguage` enum. */
const NATIVE_MENU_LOCALE_COMMAND = "set_app_menu_locale";

/** Mirror of Rust's `AppMenuLocale`, checked at runtime as well as typed. */
const NATIVE_MENU_LANGUAGES: readonly AppLanguage[] = ["en", "zh-Hans"];

let initialized = false;

/**
 * Hand the live language to the native shell so the macOS app menu is rebuilt
 * in the same language as the webview (spec FR-008 / AC-009).
 *
 * Deliberately fire-and-forget:
 * - No bridge (unit tests, plain browser runs) or no `__TAURI_INTERNALS__` →
 *   nothing to rebuild, resolve immediately.
 * - Off macOS the command is a documented no-op, so Windows and Linux pay one
 *   rejected promise at most and no code path changes.
 * - Failure is silent by design. The menu is a P1 surface; a menu that stays
 *   in the previous language must never leave the translated UI behind it.
 *
 * `language` is typed as `AppLanguage` and the Rust side deserializes a
 * two-variant enum, so no other value can reach the native menu — the
 * restriction is enforced on both ends, not by a string check.
 */
export function syncNativeMenuLocale(
  language: AppLanguage = getEffectiveLanguage(),
): Promise<void> {
  // Refuse before the IPC round trip, not just in the type: Rust would reject
  // it anyway, and a rejected command is a wasted call plus a console error.
  if (!NATIVE_MENU_LANGUAGES.includes(language)) return Promise.resolve();
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) {
    return Promise.resolve();
  }
  const notify = async (): Promise<void> => {
    // Imported lazily: i18n boots before any Tauri module is needed, and unit
    // tests must not pull the native API in to render a date.
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke(NATIVE_MENU_LOCALE_COMMAND, { locale: language });
  };
  return notify().catch(() => undefined);
}

/**
 * Boot the i18n stack before the first render:
 *
 * 1. resolve the device preference (persisted choice, else system locale),
 * 2. initialize i18next synchronously with the bundled catalogs,
 * 3. keep i18next in step with every later preference change,
 * 4. hand the effective language to the native shell (macOS app menu).
 *
 * Idempotent. Call once from the `main.tsx` bootstrap.
 */
export function initializeI18n(): void {
  if (initialized) return;
  initialized = true;

  initializeLanguagePreference();

  void i18n.use(initReactI18next).init({
    fallbackLng: FALLBACK_LANGUAGE,
    supportedLngs: ["en", "zh-Hans"],
    load: "currentOnly",
    lng: getEffectiveLanguage(),
    resources: {
      en: { translation: en },
      "zh-Hans": { translation: zhHans },
    },
    returnEmptyString: false,
    interpolation: {
      escapeValue: false,
    },
    // Resources are bundled: init synchronously so the first paint is already
    // in the resolved language (no key flash, no visible switch).
    initAsync: false,
  });

  // The native shell starts in English (or the build default); a boot into
  // zh-Hans has to hand the menu over before the first paint, not after a
  // switch that may never come.
  void syncNativeMenuLocale();

  // Runtime switches: the language module owns the effective value; i18n
  // follows it. Bundled resources make changeLanguage resolve immediately.
  subscribeLanguage(() => {
    const next = getEffectiveLanguage();
    void i18n.changeLanguage(next);
    void syncNativeMenuLocale(next);
  });
}

/**
 * React binding for components: read translation strings through
 * `useTranslation` (re-exported for a single import surface). Common
 * components must not read the global locale directly — locale comes from
 * this hook or from an explicitly passed formatter argument.
 */
export { useTranslation } from "react-i18next";
