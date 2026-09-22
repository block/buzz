/**
 * Locale model tests (spec T-001 / T-002): system-locale mapping,
 * preference parsing, and fault-tolerant storage. No DOM is required —
 * the module degrades gracefully when `document`/`window` are absent.
 */
import assert from "node:assert/strict";
import { test } from "node:test";

const language = await import("./language.ts");

// T-001: simplified-Chinese family maps to the shipped zh-Hans catalog.
test("mapSystemLocale resolves the simplified-Chinese family to zh-Hans", () => {
  for (const locale of [
    "zh-Hans",
    "zh-CN",
    "zh-SG",
    "zh-MY",
    "zh",
    "zh_cn",
    "zh-sg",
    "ZH_HANS",
    "zh-hans",
  ]) {
    assert.equal(language.mapSystemLocale(locale), "zh-Hans", locale);
  }
});

// T-001: traditional Chinese has no catalog yet — never show simplified
// silently to a traditional-locale user (spec §4.1 note).
test("mapSystemLocale falls back to en for traditional/unsupported locales", () => {
  for (const locale of [
    "zh-TW",
    "zh-HK",
    "zh-MO",
    "zh-Hant",
    "zh_hk",
    "en",
    "en-US",
    "en_gb",
    "fr",
    "fr-CA",
    "pt-BR",
    "ja",
    "ko-KR",
  ]) {
    assert.equal(language.mapSystemLocale(locale), "en", locale);
  }
});

test("resolveSystemLanguage honors preference order of the list", () => {
  assert.equal(language.resolveSystemLanguage(["fr", "zh-CN"]), "zh-Hans");
  assert.equal(language.resolveSystemLanguage(["zh-TW", "zh-CN"]), "zh-Hans");
  assert.equal(language.resolveSystemLanguage(["zh-TW", "fr"]), "en");
  assert.equal(language.resolveSystemLanguage(["en-US", "zh-CN"]), "zh-Hans");
  assert.equal(language.resolveSystemLanguage([]), "en");
  assert.equal(language.resolveSystemLanguage(null), "en");
  assert.equal(language.resolveSystemLanguage(undefined), "en");
  // Non-string / empty entries are skipped, not fatal.
  assert.equal(language.resolveSystemLanguage([null, "", "zh-SG"]), "zh-Hans");
});

// T-002: corrupt or missing stored values degrade to "system".
test("parseLanguagePreference degrades invalid values to system", () => {
  assert.equal(language.parseLanguagePreference("system"), "system");
  assert.equal(language.parseLanguagePreference("en"), "en");
  assert.equal(language.parseLanguagePreference("zh-Hans"), "zh-Hans");
  assert.equal(language.parseLanguagePreference(null), "system");
  assert.equal(language.parseLanguagePreference(undefined), "system");
  assert.equal(language.parseLanguagePreference("german"), "system");
  assert.equal(language.parseLanguagePreference("{}"), "system");
  assert.equal(language.parseLanguagePreference(""), "system");
});

function installLocalStorage(entries = {}) {
  const store = new Map(Object.entries(entries));
  const storage = {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => {
      store.set(key, String(value));
    },
    removeItem: (key) => {
      store.delete(key);
    },
  };
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: storage,
  });
  return store;
}

test("preference round-trips through storage and survives re-init", () => {
  const store = installLocalStorage();
  language.initializeLanguagePreference();
  language.setLanguagePreference("zh-Hans");
  assert.equal(store.get("buzz-language"), "zh-Hans");
  assert.equal(language.getLanguagePreference(), "zh-Hans");
  assert.equal(language.getEffectiveLanguage(), "zh-Hans");

  language.initializeLanguagePreference();
  assert.equal(language.getLanguagePreference(), "zh-Hans");

  language.setLanguagePreference("system");
  assert.equal(store.get("buzz-language"), "system");
  assert.equal(language.getLanguagePreference(), "system");
});

test("corrupt stored value degrades to the system preference", () => {
  installLocalStorage({ "buzz-language": "slavic" });
  language.initializeLanguagePreference();
  assert.equal(language.getLanguagePreference(), "system");
});

test("unreadable storage does not throw and keeps a live session switch", () => {
  const broken = {
    getItem() {
      throw new Error("localStorage denied");
    },
    setItem() {
      throw new Error("quota exceeded");
    },
  };
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: broken,
  });

  language.initializeLanguagePreference();
  assert.equal(language.getLanguagePreference(), "system");

  // The live switch still applies even though persistence failed (NFR-003).
  language.setLanguagePreference("zh-Hans");
  assert.equal(language.getEffectiveLanguage(), "zh-Hans");
  assert.equal(language.getLanguagePreference(), "zh-Hans");
});

test("system preference follows the system language list", () => {
  installLocalStorage();
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: { languages: ["zh-CN"] },
  });
  language.initializeLanguagePreference();
  assert.equal(language.getEffectiveLanguage(), "zh-Hans");

  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: { languages: ["en-US", "en"] },
  });
  language.initializeLanguagePreference();
  assert.equal(language.getEffectiveLanguage(), "en");
});

test("explicit preference wins over the system language", () => {
  installLocalStorage({ "buzz-language": "en" });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: { languages: ["zh-CN"] },
  });
  language.initializeLanguagePreference();
  assert.equal(language.getEffectiveLanguage(), "en");
});

test("<html lang> follows the effective language when a document exists", () => {
  installLocalStorage({ "buzz-language": "zh-Hans" });
  const attrs = {};
  globalThis.document = {
    documentElement: {
      setAttribute: (name, value) => {
        attrs[name] = value;
      },
    },
  };
  language.initializeLanguagePreference();
  assert.equal(attrs.lang, "zh-Hans");

  language.setLanguagePreference("en");
  assert.equal(attrs.lang, "en");
});
