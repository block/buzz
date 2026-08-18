import assert from "node:assert/strict";
import test from "node:test";

const values = new Map();
globalThis.localStorage = {
  getItem: (key) => values.get(key) ?? null,
  setItem: (key, value) => values.set(key, value),
};

const preference = await import("./linkPreviewStylePreference.ts");

test("defaults invalid and missing link preview styles to compact", () => {
  assert.equal(preference.parseLinkPreviewStyle(null), "compact");
  assert.equal(preference.parseLinkPreviewStyle("expanded"), "compact");
  assert.equal(preference.parseLinkPreviewStyle("compact"), "compact");
  assert.equal(preference.parseLinkPreviewStyle("rich"), "rich");
});

test("persists and exposes the selected link preview style", () => {
  preference.setLinkPreviewStyle("rich");
  assert.equal(preference.getLinkPreviewStyle(), "rich");
  assert.equal(values.get(preference.LINK_PREVIEW_STYLE_STORAGE_KEY), "rich");
});
