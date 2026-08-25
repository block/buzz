import assert from "node:assert/strict";
import test from "node:test";

import {
  BUSINESS_DOCK_DEFAULT_WIDTH_PX,
  BUSINESS_DOCK_MAX_WIDTH_RATIO,
  BUSINESS_DOCK_MIN_WIDTH_PX,
  BUSINESS_DOCK_WIDTH_SESSION_KEY,
  clampBusinessDockWidth,
  getBusinessDockMaxWidth,
  readBusinessDockWidth,
  saveBusinessDockWidth,
} from "./useBusinessDockWidth.ts";

test("business dock width clamps to its minimum and maximum", () => {
  assert.equal(clampBusinessDockWidth(100, 1600), BUSINESS_DOCK_MIN_WIDTH_PX);
  assert.equal(
    clampBusinessDockWidth(2000, 2000),
    2000 * BUSINESS_DOCK_MAX_WIDTH_RATIO,
  );
  assert.equal(
    clampBusinessDockWidth(BUSINESS_DOCK_DEFAULT_WIDTH_PX, 1600),
    560,
  );
});

test("business dock can occupy at most half of a desktop window", () => {
  assert.equal(getBusinessDockMaxWidth(1200), 600);
  assert.equal(getBusinessDockMaxWidth(1600), 800);
  assert.equal(getBusinessDockMaxWidth(2560), 1280);
});

test("business dock overlay can use the full width of a narrow window", () => {
  assert.equal(getBusinessDockMaxWidth(900), 900);
});

test("business dock width is restored and saved in session storage", () => {
  const values = new Map([[BUSINESS_DOCK_WIDTH_SESSION_KEY, "680"]]);
  const storage = {
    getItem(key) {
      return values.get(key) ?? null;
    },
    setItem(key, value) {
      values.set(key, value);
    },
  };

  assert.equal(readBusinessDockWidth(storage, 1600), 680);
  saveBusinessDockWidth(storage, 720);
  assert.equal(values.get(BUSINESS_DOCK_WIDTH_SESSION_KEY), "720");
});
