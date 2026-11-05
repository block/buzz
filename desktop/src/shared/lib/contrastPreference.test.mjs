import assert from "node:assert/strict";
import test from "node:test";

const values = new Map();
const attributes = new Map();
const windowListeners = new Map();

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
  },
};

const preference = await import("./contrastPreference.ts");

test("defaults invalid and missing interface contrasts to default", () => {
  assert.equal(preference.parseInterfaceContrast(null), "default");
  assert.equal(preference.parseInterfaceContrast(undefined), "default");
  assert.equal(preference.parseInterfaceContrast("flat"), "default");
  assert.equal(preference.parseInterfaceContrast("low"), "low");
  assert.equal(preference.parseInterfaceContrast("default"), "default");
  assert.equal(preference.parseInterfaceContrast("high"), "high");
});

test("persists and applies the selected interface contrast", () => {
  preference.setInterfaceContrast("low");
  assert.equal(preference.getInterfaceContrast(), "low");
  assert.equal(values.get(preference.INTERFACE_CONTRAST_STORAGE_KEY), "low");
  assert.equal(attributes.get("data-interface-contrast"), "low");

  preference.setInterfaceContrast("high");
  assert.equal(preference.getInterfaceContrast(), "high");
  assert.equal(values.get(preference.INTERFACE_CONTRAST_STORAGE_KEY), "high");
  assert.equal(attributes.get("data-interface-contrast"), "high");
});

test("previews a contrast without changing the saved preference", () => {
  preference.setInterfaceContrast("low");

  preference.previewInterfaceContrast("high");
  assert.equal(attributes.get("data-interface-contrast"), "high");
  assert.equal(preference.getInterfaceContrast(), "low");
  assert.equal(values.get(preference.INTERFACE_CONTRAST_STORAGE_KEY), "low");

  preference.previewInterfaceContrast(null);
  assert.equal(attributes.get("data-interface-contrast"), "low");
});

test("initialize applies the stored contrast and follows storage events", () => {
  values.set(preference.INTERFACE_CONTRAST_STORAGE_KEY, "high");
  preference.initializeInterfaceContrastPreference();
  assert.equal(preference.getInterfaceContrast(), "high");
  assert.equal(attributes.get("data-interface-contrast"), "high");

  values.set(preference.INTERFACE_CONTRAST_STORAGE_KEY, "low");
  windowListeners.get("storage")({
    key: preference.INTERFACE_CONTRAST_STORAGE_KEY,
  });
  assert.equal(preference.getInterfaceContrast(), "low");
  assert.equal(attributes.get("data-interface-contrast"), "low");
});

test("returns to default when another window clears storage", () => {
  preference.setInterfaceContrast("high");
  values.clear();
  windowListeners.get("storage")({ key: null });
  assert.equal(preference.getInterfaceContrast(), "default");
  assert.equal(attributes.get("data-interface-contrast"), "default");
});
