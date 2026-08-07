import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_LANGUAGE_PREFERENCE,
  readLanguagePreference,
  resolveAppLanguage,
  resolveLanguagePreference,
  resolveSystemAppLanguage,
  writeLanguagePreference,
} from "./language.ts";

test("system locale resolution recognizes simplified Chinese locales", () => {
  for (const locale of [
    "zh",
    "zh-CN",
    "zh-Hans",
    "zh-Hans-CN",
    "zh-SG",
    "zh-MY",
    "zh_CN",
  ]) {
    assert.equal(resolveSystemAppLanguage([locale]), "zh-Hans", locale);
  }
});

test("system locale resolution recognizes traditional Chinese locales", () => {
  for (const locale of ["zh-TW", "zh-Hant", "zh-Hant-TW", "zh-HK", "zh-MO"]) {
    assert.equal(resolveSystemAppLanguage([locale]), "zh-Hant", locale);
  }
});

test("unsupported system locales fall back to English", () => {
  assert.equal(resolveSystemAppLanguage(["fr-FR", "de-DE"]), "en");
  assert.equal(resolveSystemAppLanguage([]), "en");
});

test("system locale resolution uses the first supported language", () => {
  assert.equal(resolveSystemAppLanguage(["fr-FR", "zh-TW"]), "zh-Hant");
  assert.equal(resolveSystemAppLanguage(["en-US", "zh-CN"]), "en");
});

test("missing language preference means follow the system", () => {
  assert.equal(
    resolveLanguagePreference(undefined),
    DEFAULT_LANGUAGE_PREFERENCE,
  );
  assert.equal(resolveLanguagePreference("not-a-language"), "system");
  assert.equal(resolveLanguagePreference("zh-Hans"), "zh-Hans");
});

test("system preference resolves to the current system language", () => {
  assert.equal(resolveAppLanguage("system", ["zh-CN"]), "zh-Hans");
  assert.equal(resolveAppLanguage("system", ["en-US"]), "en");
  assert.equal(resolveAppLanguage("zh-Hant", ["en-US"]), "zh-Hant");
});

test("language preference persists through the settings storage seam", () => {
  const values = new Map();
  const storage = {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
  };

  assert.equal(readLanguagePreference(storage), "system");
  writeLanguagePreference(storage, "zh-Hans");
  assert.equal(readLanguagePreference(storage), "zh-Hans");
});

test("language preference survives unavailable storage", () => {
  const unavailableStorage = {
    getItem: () => {
      throw new Error("storage unavailable");
    },
    setItem: () => {
      throw new Error("storage unavailable");
    },
  };

  assert.equal(readLanguagePreference(unavailableStorage), "system");
  assert.doesNotThrow(() =>
    writeLanguagePreference(unavailableStorage, "zh-Hant"),
  );
});
