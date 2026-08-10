import assert from "node:assert/strict";
import test from "node:test";

import {
  structureShareTimelineMessages,
  timelineMessagesEqual,
} from "./structureShareTimelineMessages.ts";

function msg(overrides = {}) {
  return {
    id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    createdAt: 1_700_000_000,
    author: "alice",
    body: "hello",
    depth: 0,
    time: "12:00",
    tags: [["h", "channel"]],
    ...overrides,
  };
}

test("timelineMessagesEqual: identical values", () => {
  const a = msg();
  const b = msg();
  assert.equal(timelineMessagesEqual(a, b), true);
});

test("timelineMessagesEqual: body change", () => {
  assert.equal(timelineMessagesEqual(msg(), msg({ body: "changed" })), false);
});

test("structureShareTimelineMessages: reuses previous refs for unchanged rows", () => {
  const prevA = msg({ id: "aa".repeat(32) });
  const prevB = msg({
    id: "bb".repeat(32),
    body: "second",
  });
  const previous = [prevA, prevB];
  const next = [
    msg({ id: "aa".repeat(32) }),
    msg({ id: "bb".repeat(32), body: "second" }),
    msg({ id: "cc".repeat(32), body: "third" }),
  ];

  const shared = structureShareTimelineMessages(previous, next);
  assert.equal(shared[0], prevA);
  assert.equal(shared[1], prevB);
  assert.equal(shared[2], next[2]); // no previous match → keep next object
  assert.equal(shared.length, 3);
  assert.notEqual(shared, previous);
  assert.notEqual(shared, next);
});

test("structureShareTimelineMessages: returns previous array when fully unchanged", () => {
  const previous = [msg({ id: "aa".repeat(32) }), msg({ id: "bb".repeat(32) })];
  const next = [msg({ id: "aa".repeat(32) }), msg({ id: "bb".repeat(32) })];
  const shared = structureShareTimelineMessages(previous, next);
  assert.equal(shared, previous);
});

test("structureShareTimelineMessages: append fast path reuses prefix refs", () => {
  const prevA = msg({ id: "aa".repeat(32) });
  const prevB = msg({ id: "bb".repeat(32), body: "second" });
  const previous = [prevA, prevB];
  const next = [
    msg({ id: "aa".repeat(32) }),
    msg({ id: "bb".repeat(32), body: "second" }),
    msg({ id: "cc".repeat(32), body: "third" }),
  ];
  const shared = structureShareTimelineMessages(previous, next);
  assert.equal(shared[0], prevA);
  assert.equal(shared[1], prevB);
  assert.equal(shared[2].body, "third");
  assert.equal(shared.length, 3);
});

test("structureShareTimelineMessages: replaces row when content changes", () => {
  const prevA = msg({ id: "aa".repeat(32), body: "old" });
  const previous = [prevA];
  const next = [msg({ id: "aa".repeat(32), body: "new" })];
  const shared = structureShareTimelineMessages(previous, next);
  assert.notEqual(shared[0], prevA);
  assert.equal(shared[0].body, "new");
});
