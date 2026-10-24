import assert from "node:assert/strict";
import test from "node:test";

import { readFileSync } from "node:fs";

import config from "../../../tailwind.config.js";

const typographyCss = readFileSync(
  new URL("../styles/globals/typography.css", import.meta.url),
  "utf8",
);

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

test("derives the typography rem from the real root so zoom scales layout too", () => {
  // Cmd +/- scales the root font-size; the type rem must stay rem-relative
  // (never an absolute px) or text would zoom while containers froze.
  assert.match(
    typographyCss,
    /--buzz-type-rem:\s*calc\(1rem \* var\(--buzz-type-scale\)\);/,
  );
  assert.doesNotMatch(typographyCss, /--buzz-type-rem:\s*[\d.]+px/);
});

test("maps the font size attribute to the 13 / 14 / 15px type contract", () => {
  assert.match(typographyCss, /:root\s*\{[^}]*--buzz-type-scale:\s*1;/s);
  assert.match(
    typographyCss,
    /:root\[data-font-size="smaller"\]\s*\{\s*--buzz-type-scale:\s*calc\(13 \/ 14\);/,
  );
  assert.match(
    typographyCss,
    /:root\[data-font-size="larger"\]\s*\{\s*--buzz-type-scale:\s*calc\(15 \/ 14\);/,
  );
});

test("defaults invalid and missing font sizes to default", () => {
  assert.equal(preference.parseFontSize(null), "default");
  assert.equal(preference.parseFontSize("medium"), "default");
  assert.equal(preference.parseFontSize("smaller"), "smaller");
  assert.equal(preference.parseFontSize("default"), "default");
  assert.equal(preference.parseFontSize("larger"), "larger");
});

test("persists and applies the selected font size across the app", () => {
  preference.setFontSize("smaller");
  assert.equal(preference.getFontSize(), "smaller");
  assert.equal(values.get(preference.FONT_SIZE_STORAGE_KEY), "smaller");
  assert.equal(attributes.get("data-font-size"), "smaller");
});

test("previews a font size without changing the saved preference", () => {
  preference.setFontSize("smaller");
  preference.previewFontSize("larger");
  assert.equal(preference.getFontSize(), "smaller");
  assert.equal(values.get(preference.FONT_SIZE_STORAGE_KEY), "smaller");
  assert.equal(attributes.get("data-font-size"), "larger");

  preference.previewFontSize(null);
  assert.equal(attributes.get("data-font-size"), "smaller");
});

test("initializes from the stored font size", () => {
  values.set(preference.FONT_SIZE_STORAGE_KEY, "larger");
  preference.initializeFontSizePreference();
  assert.equal(preference.getFontSize(), "larger");
  assert.equal(attributes.get("data-font-size"), "larger");
});

test("applies font size changes from another window", () => {
  values.set(preference.FONT_SIZE_STORAGE_KEY, "smaller");
  windowListeners.get("storage")({ key: preference.FONT_SIZE_STORAGE_KEY });
  assert.equal(preference.getFontSize(), "smaller");
  assert.equal(attributes.get("data-font-size"), "smaller");
});

test("returns to the default when another window clears storage", () => {
  preference.setFontSize("larger");
  values.clear();
  windowListeners.get("storage")({ key: null });
  assert.equal(preference.getFontSize(), "default");
  assert.equal(attributes.get("data-font-size"), "default");
});
