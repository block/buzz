import assert from "node:assert/strict";
import test from "node:test";

import { buildHomeFeedLivePTagFilter } from "./useLiveHomeFeedActions.ts";
import {
  KIND_APPROVAL_REQUEST,
  KIND_REMINDER,
  KIND_TEXT_NOTE,
} from "../shared/constants/kinds.ts";

test("live p-tag filter covers Pulse mentions", () => {
  // Regression: the home feed poll pauses while the window is unfocused, so
  // Pulse (kind 1) mentions notify in the background only if this always-on
  // subscription delivers them.
  const filter = buildHomeFeedLivePTagFilter("abc123", 1_000);
  assert.ok(filter.kinds.includes(KIND_TEXT_NOTE));
});

test("live p-tag filter keeps approval and reminder coverage", () => {
  const filter = buildHomeFeedLivePTagFilter("abc123", 1_000);
  assert.ok(filter.kinds.includes(KIND_APPROVAL_REQUEST));
  assert.ok(filter.kinds.includes(KIND_REMINDER));
});

test("live p-tag filter targets the user and is live-only", () => {
  const filter = buildHomeFeedLivePTagFilter("abc123", 1_234);
  assert.deepEqual(filter["#p"], ["abc123"]);
  assert.equal(filter.since, 1_234);
});
