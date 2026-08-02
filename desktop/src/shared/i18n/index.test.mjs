import assert from "node:assert/strict";
import test from "node:test";

import {
  detectInitialLanguage,
  LANGUAGE_STORAGE_KEY,
  normalizeLanguage,
} from "./language.ts";

test("normalizeLanguage_mapsPortugueseVariantsToPtBR", () => {
  assert.equal(normalizeLanguage("pt-PT"), "pt-BR");
  assert.equal(normalizeLanguage("pt-BR"), "pt-BR");
  assert.equal(normalizeLanguage("en-GB"), "en-US");
});

test("detectInitialLanguage_prefersPersistedSupportedLocale", () => {
  const storage = {
    getItem(key) {
      assert.equal(key, LANGUAGE_STORAGE_KEY);
      return "en-US";
    },
  };
  assert.equal(detectInitialLanguage(storage, "pt-BR"), "en-US");
});

test("detectInitialLanguage_fallsBackToBrowserLocale", () => {
  const storage = { getItem: () => null };
  assert.equal(detectInitialLanguage(storage, "pt-BR"), "pt-BR");
});
