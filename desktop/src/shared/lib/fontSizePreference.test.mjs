import assert from "node:assert/strict";
import test from "node:test";

import config from "../../../tailwind.config.js";

const values = new Map();
const attributes = new Map();
const styleValues = new Map();
const windowListeners = new Map();
const style = {
  setProperty: (name, value) => styleValues.set(name, value),
};

globalThis.window = {
  addEventListener: (type, listener) => windowListeners.set(type, listener),
};
globalThis.localStorage = {
  getItem: (key) => values.get(key) ?? null,
  setItem: (key, value) => values.set(key, String(value)),
};
globalThis.document = {
  documentElement: {
    setAttribute: (name, value) => attributes.set(name, value),
    style,
  },
};

const preference = await import("./fontSizePreference.ts");

test("scales fixed line-height utilities with the typography rem", () => {
  assert.deepEqual(config.theme.extend.lineHeight, {
    3: "calc(var(--buzz-type-rem) * 0.75)",
    4: "var(--buzz-type-rem)",
    5: "calc(var(--buzz-type-rem) * 1.25)",
    6: "calc(var(--buzz-type-rem) * 1.5)",
    7: "calc(var(--buzz-type-rem) * 1.75)",
    8: "calc(var(--buzz-type-rem) * 2)",
    "message-author": "var(--conversation-author-line-height)",
  });
});

test("defaults invalid and missing font sizes to default", () => {
  assert.equal(preference.parseFontSize(null), "default");
  assert.equal(preference.parseFontSize("medium"), "default");
  assert.equal(preference.parseFontSize("smaller"), "smaller");
  assert.equal(preference.parseFontSize("default"), "default");
  assert.equal(preference.parseFontSize("larger"), "larger");
});

test("persists and applies the selected font size across the app", () => {
  preference.applyTextZoomFactor(1);
  preference.setFontSize("smaller");
  assert.equal(preference.getFontSize(), "smaller");
  assert.equal(values.get(preference.FONT_SIZE_STORAGE_KEY), "smaller");
  assert.equal(attributes.get("data-font-size"), "smaller");
  assert.equal(styleValues.get("--buzz-type-rem"), "14.857143px");
});

test("previews a font size without changing the saved preference", () => {
  preference.applyTextZoomFactor(1.1);
  preference.setFontSize("smaller");
  preference.previewFontSize("larger");
  assert.equal(preference.getFontSize(), "smaller");
  assert.equal(values.get(preference.FONT_SIZE_STORAGE_KEY), "smaller");
  assert.equal(attributes.get("data-font-size"), "larger");
  assert.equal(styleValues.get("--buzz-type-rem"), "18.857143px");

  preference.previewFontSize(null);
  assert.equal(attributes.get("data-font-size"), "smaller");
  assert.equal(styleValues.get("--buzz-type-rem"), "16.342857px");
});

test("initializes from the stored font size", () => {
  preference.applyTextZoomFactor(1);
  values.set(preference.FONT_SIZE_STORAGE_KEY, "larger");
  preference.initializeFontSizePreference();
  assert.equal(preference.getFontSize(), "larger");
  assert.equal(attributes.get("data-font-size"), "larger");
  assert.equal(styleValues.get("--buzz-type-rem"), "17.142857px");
});

test("applies font size changes from another window", () => {
  values.set(preference.FONT_SIZE_STORAGE_KEY, "smaller");
  windowListeners.get("storage")({ key: preference.FONT_SIZE_STORAGE_KEY });
  assert.equal(preference.getFontSize(), "smaller");
  assert.equal(attributes.get("data-font-size"), "smaller");
  assert.equal(styleValues.get("--buzz-type-rem"), "14.857143px");
});

test("returns to the default when another window clears storage", () => {
  preference.setFontSize("larger");
  values.clear();
  windowListeners.get("storage")({ key: null });
  assert.equal(preference.getFontSize(), "default");
  assert.equal(attributes.get("data-font-size"), "default");
  assert.equal(styleValues.get("--buzz-type-rem"), "16px");
});
