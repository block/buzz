import assert from "node:assert/strict";
import test from "node:test";

import {
  HUDDLE_JOINABLE_WINDOW_SECONDS,
  isHuddleStartStale,
} from "./huddleCardState.ts";

test("isHuddleStartStale keeps recent huddle cards joinable", () => {
  const nowSeconds = 2_000_000;
  assert.equal(
    isHuddleStartStale(
      nowSeconds - HUDDLE_JOINABLE_WINDOW_SECONDS + 1,
      nowSeconds * 1000,
    ),
    false,
  );
});

test("isHuddleStartStale marks huddle cards stale after the joinable window", () => {
  const nowSeconds = 2_000_000;
  assert.equal(
    isHuddleStartStale(
      nowSeconds - HUDDLE_JOINABLE_WINDOW_SECONDS - 1,
      nowSeconds * 1000,
    ),
    true,
  );
});
