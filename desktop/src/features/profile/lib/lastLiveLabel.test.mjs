import assert from "node:assert/strict";
import test from "node:test";

import { formatLastLiveLabel } from "./lastLiveLabel.ts";

const NOW = Date.parse("2026-07-23T12:00:00.000Z");

test("formatLastLiveLabel reports missing activity", () => {
  assert.equal(formatLastLiveLabel(null, NOW), "No activity yet");
});

test("formatLastLiveLabel clamps future timestamps to Just now", () => {
  assert.equal(formatLastLiveLabel(NOW + 5_000, NOW), "Just now");
});

test("formatLastLiveLabel buckets by elapsed time", () => {
  assert.equal(formatLastLiveLabel(NOW - 30 * 1000, NOW), "Just now");
  assert.equal(formatLastLiveLabel(NOW - 5 * 60 * 1000, NOW), "5m ago");
  assert.equal(formatLastLiveLabel(NOW - 3 * 60 * 60 * 1000, NOW), "3h ago");
  assert.equal(
    formatLastLiveLabel(NOW - 2 * 24 * 60 * 60 * 1000, NOW),
    "2d ago",
  );
  assert.equal(
    formatLastLiveLabel(NOW - 3 * 7 * 24 * 60 * 60 * 1000, NOW),
    "3w ago",
  );
});
