import assert from "node:assert/strict";
import test from "node:test";

import {
  MESSAGE_GROUPING_WINDOW_SECONDS,
  hasSameMessageAuthor,
  isWithinGroupingWindow,
} from "./messageGrouping.ts";

test("hasSameMessageAuthor: matches case-insensitively and trims", () => {
  assert.equal(
    hasSameMessageAuthor({ pubkey: " ABC " }, { pubkey: "abc" }),
    true,
  );
  assert.equal(
    hasSameMessageAuthor({ pubkey: "abc" }, { pubkey: "def" }),
    false,
  );
});

test("hasSameMessageAuthor: missing pubkeys never match", () => {
  assert.equal(hasSameMessageAuthor(null, { pubkey: "abc" }), false);
  assert.equal(hasSameMessageAuthor({ pubkey: "abc" }, undefined), false);
  assert.equal(hasSameMessageAuthor({ pubkey: "" }, { pubkey: "" }), false);
});

test("isWithinGroupingWindow: at or under the boundary is in window", () => {
  const base = 1_000_000;
  assert.equal(isWithinGroupingWindow(base, base), true);
  assert.equal(
    isWithinGroupingWindow(base, base + MESSAGE_GROUPING_WINDOW_SECONDS),
    true,
  );
});

test("isWithinGroupingWindow: past the boundary is out of window", () => {
  const base = 1_000_000;
  assert.equal(
    isWithinGroupingWindow(base, base + MESSAGE_GROUPING_WINDOW_SECONDS + 1),
    false,
  );
});

test("isWithinGroupingWindow: out-of-order (negative gap) is out of window", () => {
  const base = 1_000_000;
  assert.equal(isWithinGroupingWindow(base + 60, base), false);
});

test("isWithinGroupingWindow: missing timestamps are out of window", () => {
  assert.equal(isWithinGroupingWindow(null, 1_000_000), false);
  assert.equal(isWithinGroupingWindow(1_000_000, undefined), false);
  assert.equal(isWithinGroupingWindow(undefined, undefined), false);
});
