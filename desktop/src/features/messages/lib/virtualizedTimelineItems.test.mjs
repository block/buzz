import assert from "node:assert/strict";
import test from "node:test";

import {
  buildVirtualizedItems,
  didPrependVirtualizedTimeline,
  estimateVirtualizedTimelineItemHeight,
  virtualizedItemKey,
} from "./virtualizedTimelineItems.ts";

// Timestamps: day A = 2026-06-01, day B = 2026-06-02, day Z = 2026-05-31.
function dayAt(year, month, day, hour = 12, minute = 0) {
  return Math.floor(
    new Date(year, month - 1, day, hour, minute, 0).getTime() / 1_000,
  );
}

function messageItem(key) {
  return {
    kind: "message",
    key,
    entry: { message: { id: key } },
    isContinuation: false,
    isFollowedByContinuation: false,
  };
}

function group(dayKey, headingTimestamp, itemKeys) {
  return {
    key: dayKey,
    headingTimestamp,
    items: itemKeys.map(messageItem),
  };
}

const DAY_A = dayAt(2026, 6, 1);
const DAY_B = dayAt(2026, 6, 2);
const DAY_Z = dayAt(2026, 5, 31);

function keysOf(dayGroups, { leading = undefined, exhausted = false } = {}) {
  return buildVirtualizedItems(dayGroups, leading, exhausted).map(
    virtualizedItemKey,
  );
}

/**
 * Virtua's `shift` moves every cached slot uniformly by the length delta.
 * With shift admitted, assert every previous key's cached height lands on the
 * SAME logical item in the new list.
 */
function assertShiftAdmittedAndCacheClean(previousKeys, keys) {
  assert.equal(didPrependVirtualizedTimeline(previousKeys, keys), true);
  const delta = keys.length - previousKeys.length;
  previousKeys.forEach((key, index) => {
    assert.equal(
      keys[index + delta],
      key,
      `cached height at slot ${index} (${key}) would land on ${keys[index + delta]}`,
    );
  });
}

test("oldest day carries no divider while more history exists", () => {
  const keys = keysOf([group("day-A", DAY_A, ["a3", "a4", "a5"])]);
  assert.deepEqual(keys, ["a3", "a4", "a5", "bottom-spacer"]);
});

test("oldest day divider renders once history is exhausted", () => {
  const keys = keysOf([group("day-A", DAY_A, ["a3", "a4", "a5"])], {
    exhausted: true,
  });
  assert.deepEqual(keys, [
    "day-divider:day-A",
    "a3",
    "a4",
    "a5",
    "bottom-spacer",
  ]);
});

test("interior day boundaries always render dividers", () => {
  const keys = keysOf([
    group("day-A", DAY_A, ["a3"]),
    group("day-B", DAY_B, ["b1", "b2"]),
  ]);
  assert.deepEqual(keys, [
    "a3",
    "day-divider:day-B",
    "b1",
    "b2",
    "bottom-spacer",
  ]);
});

test("undated group never renders a divider even when proven", () => {
  const keys = keysOf(
    [group("day-undated", null, ["u1"]), group("day-B", DAY_B, ["b1"])],
    { exhausted: true },
  );
  assert.deepEqual(keys, ["u1", "day-divider:day-B", "b1", "bottom-spacer"]);
});

test("same-day prepend into the oldest day admits shift with a clean cache", () => {
  const previous = keysOf([
    group("day-A", DAY_A, ["a3", "a4", "a5"]),
    group("day-B", DAY_B, ["b1"]),
  ]);
  const next = keysOf([
    group("day-A", DAY_A, ["a1", "a2", "a3", "a4", "a5"]),
    group("day-B", DAY_B, ["b1"]),
  ]);
  assertShiftAdmittedAndCacheClean(previous, next);
});

test("cross-day prepend materializes the newly proven divider as pure prefix", () => {
  const previous = keysOf([
    group("day-A", DAY_A, ["a3", "a4", "a5"]),
    group("day-B", DAY_B, ["b1"]),
  ]);
  // Day Z loads before day A: day A's boundary is now proven, so BOTH the
  // z-rows and day A's divider enter as prefix.
  const next = keysOf([
    group("day-Z", DAY_Z, ["z1", "z2"]),
    group("day-A", DAY_A, ["a3", "a4", "a5"]),
    group("day-B", DAY_B, ["b1"]),
  ]);
  assert.deepEqual(
    next.slice(0, 4),
    ["z1", "z2", "day-divider:day-A", "a3"],
    "day-Z itself is now the unproven oldest day and carries no divider",
  );
  assertShiftAdmittedAndCacheClean(previous, next);
});

test("mixed page (older day rows + same-day completion) admits shift", () => {
  const previous = keysOf([
    group("day-A", DAY_A, ["a3", "a4", "a5"]),
    group("day-B", DAY_B, ["b1"]),
  ]);
  const next = keysOf([
    group("day-Z", DAY_Z, ["z9"]),
    group("day-A", DAY_A, ["a1", "a2", "a3", "a4", "a5"]),
    group("day-B", DAY_B, ["b1"]),
  ]);
  assertShiftAdmittedAndCacheClean(previous, next);
});

test("history exhaustion with an empty page inserts the divider as pure prefix", () => {
  const groups = [
    group("day-A", DAY_A, ["a3", "a4", "a5"]),
    group("day-B", DAY_B, ["b1"]),
  ];
  const previous = keysOf(groups);
  const next = keysOf(groups, { exhausted: true });
  assert.deepEqual(next.slice(0, 2), ["day-divider:day-A", "a3"]);
  assertShiftAdmittedAndCacheClean(previous, next);
});

test("leading content arriving with the final page stays a pure prefix", () => {
  const previous = keysOf([
    group("day-A", DAY_A, ["a3", "a4", "a5"]),
    group("day-B", DAY_B, ["b1"]),
  ]);
  const next = keysOf(
    [
      group("day-A", DAY_A, ["a1", "a2", "a3", "a4", "a5"]),
      group("day-B", DAY_B, ["b1"]),
    ],
    { leading: "intro", exhausted: true },
  );
  assert.deepEqual(next.slice(0, 2), ["leading-content", "day-divider:day-A"]);
  assertShiftAdmittedAndCacheClean(previous, next);
});

test("divider count and order match proven boundaries exactly", () => {
  const items = buildVirtualizedItems(
    [
      group("day-Z", DAY_Z, ["z1"]),
      group("day-A", DAY_A, ["a1"]),
      group("day-B", DAY_B, ["b1"]),
    ],
    undefined,
    true,
  );
  const dividers = items.filter((item) => item.kind === "day-divider");
  assert.deepEqual(
    dividers.map((item) => item.key),
    ["day-divider:day-Z", "day-divider:day-A", "day-divider:day-B"],
  );
  // Each divider precedes its day's first message.
  const keys = items.map(virtualizedItemKey);
  assert.equal(keys.indexOf("day-divider:day-Z"), keys.indexOf("z1") - 1);
  assert.equal(keys.indexOf("day-divider:day-A"), keys.indexOf("a1") - 1);
  assert.equal(keys.indexOf("day-divider:day-B"), keys.indexOf("b1") - 1);
});

test("naive counterexample stays rejected: same key cannot precede prepended rows", () => {
  // Regression pin for the shape Wren vetoed: if the oldest day's divider
  // existed BEFORE the same-day prepend, admission must fail.
  assert.equal(
    didPrependVirtualizedTimeline(
      ["day-divider:day-A", "a3", "a4", "a5", "bottom-spacer"],
      ["day-divider:day-A", "a1", "a2", "a3", "a4", "a5", "bottom-spacer"],
    ),
    false,
  );
});

test("virtualized rows preserve their heterogeneous height estimates", () => {
  const short = messageItem("short");
  short.entry.message.body = "hello";
  const tall = messageItem("tall");
  tall.entry.message.body = Array.from(
    { length: 20 },
    (_, index) => `line ${index}`,
  ).join("\n");
  const items = buildVirtualizedItems(
    [{ key: "day-A", headingTimestamp: DAY_A, items: [short, tall] }],
    undefined,
    true,
  );
  const estimates = items.map(estimateVirtualizedTimelineItemHeight);

  assert.equal(estimates[0], 32);
  assert.ok(estimates[2] > estimates[1] + 200);
  assert.equal(estimates.at(-1), 96);
});
