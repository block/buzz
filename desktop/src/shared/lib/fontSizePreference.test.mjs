import assert from "node:assert/strict";
import test from "node:test";

const values = new Map();
const attributes = new Map();
const style = { fontSize: "" };

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
  assert.equal(style.fontSize, "15px");
});

test("previews a font size without changing the saved preference", () => {
  preference.applyTextZoomFactor(1.1);
  preference.setFontSize("smaller");
  preference.previewFontSize("larger");
  assert.equal(preference.getFontSize(), "smaller");
  assert.equal(values.get(preference.FONT_SIZE_STORAGE_KEY), "smaller");
  assert.equal(attributes.get("data-font-size"), "larger");
  assert.equal(style.fontSize, "18.7px");

  preference.previewFontSize(null);
  assert.equal(attributes.get("data-font-size"), "smaller");
  assert.equal(style.fontSize, "16.5px");
});

test("initializes from the stored font size", () => {
  preference.applyTextZoomFactor(1);
  values.set(preference.FONT_SIZE_STORAGE_KEY, "larger");
  preference.initializeFontSizePreference();
  assert.equal(preference.getFontSize(), "larger");
  assert.equal(attributes.get("data-font-size"), "larger");
  assert.equal(style.fontSize, "17px");
});
