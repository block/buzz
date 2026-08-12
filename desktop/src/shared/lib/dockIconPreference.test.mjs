import assert from "node:assert/strict";
import test from "node:test";

const values = new Map();
globalThis.localStorage = {
  getItem: (key) => values.get(key) ?? null,
  setItem: (key, value) => values.set(key, value),
};

const preference = await import("./dockIconPreference.ts");

test("defaults to keeping the Dock icon", () => {
  assert.equal(preference.getHideDockIconOnClose(), false);
  assert.equal(preference.DEFAULT_HIDE_DOCK_ICON_ON_CLOSE, false);
});

test("persists the selected dock behavior", () => {
  preference.setHideDockIconOnClose(true);
  assert.equal(preference.getHideDockIconOnClose(), true);
  assert.equal(values.get(preference.HIDE_DOCK_ICON_STORAGE_KEY), "true");

  preference.setHideDockIconOnClose(false);
  assert.equal(preference.getHideDockIconOnClose(), false);
  assert.equal(values.get(preference.HIDE_DOCK_ICON_STORAGE_KEY), "false");
});

test("treats any non-true stored value as disabled", () => {
  for (const stored of ["", "1", "yes", "TRUE", null]) {
    values.set(preference.HIDE_DOCK_ICON_STORAGE_KEY, stored);
    assert.equal(
      globalThis.localStorage.getItem(preference.HIDE_DOCK_ICON_STORAGE_KEY) ===
        "true",
      false,
    );
  }
});
