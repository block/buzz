import assert from "node:assert/strict";
import test from "node:test";

import { formatHorizontalResizeIndicator } from "./startHorizontalMouseResize.ts";

test("formats resize width and rounded container percentage", () => {
  assert.equal(formatHorizontalResizeIndicator(560, 1600), "560 px · 35%");
  assert.equal(formatHorizontalResizeIndicator(421.6, 1200), "422 px · 35%");
});

test("uses zero percent when the container has no measurable width", () => {
  assert.equal(formatHorizontalResizeIndicator(560, 0), "560 px · 0%");
});
