import assert from "node:assert/strict";
import test from "node:test";

import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { buildTimelineItems, getTimelineItemKey } from "./timelineItems.ts";

function dayAt(year, month, day, hour = 12) {
  return Math.floor(
    new Date(year, month - 1, day, hour, 0, 0).getTime() / 1_000,
  );
}

function message(overrides) {
  return {
    id: "m",
    renderKey: undefined,
    createdAt: dayAt(2026, 6, 14),
    pubkey: "author",
    parentId: null,
    rootId: null,
    depth: 0,
    kind: 9,
    tags: [],
    ...overrides,
  };
}

// The builder takes MainTimelineEntry[] (post top-level filter); summary is
// irrelevant to item/divider placement, so null is fine here.
function entry(overrides) {
  return { message: message(overrides), summary: null };
}

function kinds(items) {
  return items.map((item) => item.kind);
}

// --- divider placement -------------------------------------------------------

test("buildTimelineItems: 3-day channel with unread mid-day-2 places dividers by index", () => {
  const entries = [
    entry({ id: "d1a", createdAt: dayAt(2026, 6, 12) }),
    entry({ id: "d1b", createdAt: dayAt(2026, 6, 12, 13) }),
    entry({ id: "d2a", createdAt: dayAt(2026, 6, 13) }),
    entry({ id: "d2b", createdAt: dayAt(2026, 6, 13, 13) }), // first unread
    entry({ id: "d2c", createdAt: dayAt(2026, 6, 13, 14) }),
    entry({ id: "d3a", createdAt: dayAt(2026, 6, 14) }),
  ];

  const { items } = buildTimelineItems(entries, "d2b");

  assert.deepEqual(kinds(items), [
    "day-divider", // day 1
    "message", // d1a
    "message", // d1b
    "day-divider", // day 2
    "message", // d2a
    "unread-divider", // above d2b
    "message", // d2b
    "message", // d2c
    "day-divider", // day 3
    "message", // d3a
  ]);
});

test("buildTimelineItems: unread divider suppressed when first unread is the first entry", () => {
  const entries = [
    entry({ id: "a", createdAt: dayAt(2026, 6, 14) }),
    entry({ id: "b", createdAt: dayAt(2026, 6, 14, 13) }),
  ];
  // firstUnread === index 0 — nothing above it, so no divider.
  const { items } = buildTimelineItems(entries, "a");
  assert.equal(items.filter((i) => i.kind === "unread-divider").length, 0);
});

test("buildTimelineItems: system messages flatten to a 'system' item", () => {
  const entries = [
    entry({ id: "a", createdAt: dayAt(2026, 6, 14) }),
    entry({
      id: "sys",
      kind: KIND_SYSTEM_MESSAGE,
      createdAt: dayAt(2026, 6, 14, 13),
    }),
  ];
  const { items } = buildTimelineItems(entries, null);
  assert.deepEqual(kinds(items), ["day-divider", "message", "system"]);
});

test("buildTimelineItems: empty entries produce no items and an empty map", () => {
  const { items, indexByMessageId } = buildTimelineItems([], null);
  assert.equal(items.length, 0);
  assert.equal(indexByMessageId.size, 0);
});

// --- index map correctness ---------------------------------------------------

test("buildTimelineItems: map points each message id at its flattened item index", () => {
  const entries = [
    entry({ id: "d1", createdAt: dayAt(2026, 6, 12) }),
    entry({ id: "d2", createdAt: dayAt(2026, 6, 13) }),
  ];
  const { items, indexByMessageId } = buildTimelineItems(entries, null);

  // dividers occupy indices 0 and 2; messages land at 1 and 3.
  assert.equal(indexByMessageId.get("d1"), 1);
  assert.equal(indexByMessageId.get("d2"), 3);
  assert.equal(items[1].entry.message.id, "d1");
  assert.equal(items[3].entry.message.id, "d2");
});

test("buildTimelineItems: appending a new message keeps prior indices stable", () => {
  const base = [entry({ id: "a", createdAt: dayAt(2026, 6, 14) })];
  const before = buildTimelineItems(base, null).indexByMessageId;

  const appended = [
    ...base,
    entry({ id: "b", createdAt: dayAt(2026, 6, 14, 13) }),
  ];
  const after = buildTimelineItems(appended, null).indexByMessageId;

  assert.equal(after.get("a"), before.get("a"));
  assert.equal(after.get("b"), 2);
});

test("buildTimelineItems: prepending an older-day message shifts later indices", () => {
  const original = [entry({ id: "b", createdAt: dayAt(2026, 6, 14) })];
  const beforeIdx = buildTimelineItems(original, null).indexByMessageId.get(
    "b",
  );

  // Prepend a message on an earlier day → adds its own day-divider + message,
  // pushing "b" (now on a new day boundary too) further down.
  const prepended = [
    entry({ id: "a", createdAt: dayAt(2026, 6, 13) }),
    entry({ id: "b", createdAt: dayAt(2026, 6, 14) }),
  ];
  const afterIdx = buildTimelineItems(prepended, null).indexByMessageId.get(
    "b",
  );
  assert.ok(afterIdx > beforeIdx);
});

test("buildTimelineItems: deleting a message drops it from the map", () => {
  const entries = [
    entry({ id: "a", createdAt: dayAt(2026, 6, 14) }),
    entry({ id: "b", createdAt: dayAt(2026, 6, 14, 13) }),
  ];
  const afterDelete = buildTimelineItems(
    entries.filter((e) => e.message.id !== "a"),
    null,
  ).indexByMessageId;
  assert.equal(afterDelete.has("a"), false);
  assert.equal(afterDelete.get("b"), 1);
});

test("getTimelineItemKey: keys are unique across the stream", () => {
  const entries = [
    entry({ id: "a", createdAt: dayAt(2026, 6, 12) }),
    entry({ id: "b", createdAt: dayAt(2026, 6, 13) }),
  ];
  const { items } = buildTimelineItems(entries, "b");
  const keys = items.map(getTimelineItemKey);
  assert.equal(new Set(keys).size, keys.length);
});
