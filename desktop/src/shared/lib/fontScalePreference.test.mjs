import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  FONT_SCALE_PRESETS,
  getFontScale,
  setFontScale,
} from "./fontScalePreference.ts";

function installLocalStorage() {
  const store = new Map();
  const ls = {
    getItem: (key) => store.get(key) ?? null,
    removeItem: (key) => store.delete(key),
    setItem(key, value) {
      store.set(key, value);
    },
  };
  if (typeof globalThis.window === "undefined") {
    globalThis.window = {};
  }
  globalThis.window.localStorage = ls;
  globalThis.localStorage = ls;
  return store;
}

describe("fontScalePreference", () => {
  it("clamps the stored value into the supported 85%-130% range", () => {
    installLocalStorage();
    setFontScale(0.5);
    assert.equal(getFontScale(), FONT_SCALE_PRESETS[0]);

    setFontScale(1.4);
    assert.equal(getFontScale(), FONT_SCALE_PRESETS[FONT_SCALE_PRESETS.length - 1]);
  });

  it("persists the exact finite value across reads", () => {
    installLocalStorage();
    setFontScale(1.15);
    assert.equal(getFontScale(), 1.15);
  });

  it("round-trips through localStorage when available", () => {
    const store = installLocalStorage();
    setFontScale(1.2);
    assert.equal(store.get("buzz.ui.fontScale"), "1.2");
  });

  it("resets to 100% for NaN input", () => {
    installLocalStorage();
    setFontScale(Number.NaN);
    assert.equal(getFontScale(), 1);
  });
});
