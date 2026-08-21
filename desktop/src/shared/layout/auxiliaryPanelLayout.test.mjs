import assert from "node:assert/strict";
import test from "node:test";

import {
  AUXILIARY_PANEL_MAX_WIDTH_PX,
  AUXILIARY_PANEL_MIN_WIDTH_PX,
  clampAuxiliaryPanelWidth,
  getAuxiliaryPanelMaxWidth,
} from "./auxiliaryPanelLayout.ts";

test("max width falls back to the static cap on narrow viewports", () => {
  // On viewports where `viewportWidth - MIN` is below the static cap, the floor wins.
  assert.equal(getAuxiliaryPanelMaxWidth(900), AUXILIARY_PANEL_MAX_WIDTH_PX);
  assert.equal(getAuxiliaryPanelMaxWidth(0), AUXILIARY_PANEL_MAX_WIDTH_PX);
});

test("max width grows with the viewport, reserving the main pane", () => {
  assert.equal(
    getAuxiliaryPanelMaxWidth(3440),
    3440 - AUXILIARY_PANEL_MIN_WIDTH_PX,
  );
});

test("clamp keeps width within [min, viewport-aware max]", () => {
  // Below the floor clamps up to the min.
  assert.equal(
    clampAuxiliaryPanelWidth(100, 3440),
    AUXILIARY_PANEL_MIN_WIDTH_PX,
  );
  // A wide drag is allowed on an ultrawide viewport.
  assert.equal(clampAuxiliaryPanelWidth(2000, 3440), 2000);
  // The same wide value is clamped down on a small viewport.
  assert.equal(
    clampAuxiliaryPanelWidth(2000, 900),
    AUXILIARY_PANEL_MAX_WIDTH_PX,
  );
});
