import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  DEFAULT_LANG,
  LANG_STORAGE_KEY,
  isLang,
  loadStoredLang,
  messages,
  persistLang,
  translate,
} from "./messages.ts";

describe("desktop i18n", () => {
  it("defaults to Chinese", () => {
    assert.equal(DEFAULT_LANG, "zh");
  });

  it("recognizes supported langs only", () => {
    assert.equal(isLang("zh"), true);
    assert.equal(isLang("en"), true);
    assert.equal(isLang("fr"), false);
    assert.equal(isLang(null), false);
  });

  it("has matching keys in en and zh", () => {
    const enKeys = Object.keys(messages.en).sort();
    const zhKeys = Object.keys(messages.zh).sort();
    assert.deepEqual(enKeys, zhKeys);
  });

  it("translates main-path keys", () => {
    assert.equal(translate("nav.inbox", "en"), "Inbox");
    assert.equal(translate("nav.inbox", "zh"), "收件箱");
    assert.equal(translate("nav.channels", "zh"), "频道");
    assert.equal(translate("appearance.language.title", "zh"), "语言");
    assert.equal(translate("settings.backToApp", "zh"), "返回应用");
    assert.equal(translate("search.noRecentActivity", "en"), "No recent activity yet.");
    assert.equal(
      translate("search.noMatchesFor", "zh", { query: "foo" }),
      "无匹配：foo。",
    );
  });

  it("falls back to English for missing zh (defensive)", () => {
    assert.equal(translate("common.save", "en"), "Save");
    assert.equal(translate("common.save", "zh"), "保存");
  });

  it("loadStoredLang defaults when empty and reads valid values", () => {
    const store = new Map();
    const storage = {
      getItem: (k) => (store.has(k) ? store.get(k) : null),
      setItem: (k, v) => {
        store.set(k, v);
      },
    };
    assert.equal(loadStoredLang(storage), DEFAULT_LANG);
    storage.setItem(LANG_STORAGE_KEY, "en");
    assert.equal(loadStoredLang(storage), "en");
    storage.setItem(LANG_STORAGE_KEY, "bogus");
    assert.equal(loadStoredLang(storage), DEFAULT_LANG);
  });

  it("persistLang writes storage so a later load keeps the choice", () => {
    const store = new Map();
    const storage = {
      getItem: (k) => (store.has(k) ? store.get(k) : null),
      setItem: (k, v) => {
        store.set(k, v);
      },
    };
    assert.equal(persistLang(storage, "en"), true);
    assert.equal(store.get(LANG_STORAGE_KEY), "en");
    assert.equal(loadStoredLang(storage), "en");
    assert.equal(persistLang(storage, "zh"), true);
    assert.equal(loadStoredLang(storage), "zh");
  });
});
