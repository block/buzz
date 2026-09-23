import assert from "node:assert/strict";
import test from "node:test";

import { resolveDecisionDeadlineSecs } from "./permissionDecisionDelivery.ts";

const NOW = 1_700_000_000;

test("resolveDecisionDeadlineSecs prefers the card's own expiresAt", () => {
  assert.equal(
    resolveDecisionDeadlineSecs(1_700_000_300, "2026-08-28T00:00:00Z", NOW),
    1_700_000_300,
    "an explicit expiresAt wins over any fallback",
  );
});

test("resolveDecisionDeadlineSecs falls back to frame timestamp + 300s when expiresAt is absent", () => {
  // A pre-upgrade/archived frame without expiresAt anchors on the frame's own
  // clock, not click time — a long-archived card is already past its deadline.
  const ts = "2023-11-14T22:13:20.000Z"; // == 1_700_000_000 unix seconds
  assert.equal(
    resolveDecisionDeadlineSecs(undefined, ts, NOW + 999_999),
    NOW + 300,
    "deadline = frame-timestamp seconds + 300, independent of now",
  );
});

test("resolveDecisionDeadlineSecs falls back to now + 300s when the timestamp is unparseable", () => {
  assert.equal(
    resolveDecisionDeadlineSecs(undefined, "not-a-date", NOW),
    NOW + 300,
    "an unparseable timestamp anchors on now so the loop still terminates",
  );
  assert.equal(
    resolveDecisionDeadlineSecs(undefined, undefined, NOW),
    NOW + 300,
    "an absent timestamp anchors on now",
  );
});
