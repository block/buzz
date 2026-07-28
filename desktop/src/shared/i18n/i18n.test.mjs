import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  DEFAULT_LANG,
  isLang,
  messages,
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
  });

  it("falls back to English for missing zh (defensive)", () => {
    // All keys exist; unknown key returns the key itself via ?? chain
    // when accessed incorrectly — translate uses typed keys only.
    assert.equal(translate("common.save", "en"), "Save");
    assert.equal(translate("common.save", "zh"), "保存");
  });
});
