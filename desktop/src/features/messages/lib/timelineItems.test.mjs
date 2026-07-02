import assert from "node:assert/strict";
import test from "node:test";

import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import {
  buildTimelineDayGroups,
  buildTimelineItems,
  getTimelineItemKey,
} from "./timelineItems.ts";

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

test("buildTimelineItems: consecutive messages from the same author are grouped", () => {
  const entries = [
    entry({ id: "a", pubkey: "author-a", createdAt: dayAt(2026, 6, 14) }),
    entry({ id: "b", pubkey: "AUTHOR-A", createdAt: dayAt(2026, 6, 14, 13) }),
    entry({ id: "c", pubkey: "author-b", createdAt: dayAt(2026, 6, 14, 14) }),
  ];

  const messageItems = buildTimelineItems(entries, null).items.filter(
    (item) => item.kind === "message",
  );

  assert.deepEqual(
    messageItems.map((item) => item.isContinuation),
    [false, true, false],
  );
  assert.deepEqual(
    messageItems.map((item) => item.isFollowedByContinuation),
    [true, false, false],
  );
});

test("buildTimelineItems: dividers break grouping while thread summaries do not", () => {
  const sameAuthor = "author-a";
  const entries = [
    entry({ id: "a", pubkey: sameAuthor, createdAt: dayAt(2026, 6, 14) }),
    {
      ...entry({
        id: "b",
        pubkey: sameAuthor,
        createdAt: dayAt(2026, 6, 14, 13),
      }),
      summary: {
        threadHeadId: "b",
        replyCount: 1,
        lastReplyAt: dayAt(2026, 6, 14, 13),
        participants: [],
      },
    },
    entry({ id: "c", pubkey: sameAuthor, createdAt: dayAt(2026, 6, 14, 14) }),
    entry({ id: "d", pubkey: sameAuthor, createdAt: dayAt(2026, 6, 15) }),
  ];

  const messageItems = buildTimelineItems(entries, "c").items.filter(
    (item) => item.kind === "message",
  );

  assert.deepEqual(
    messageItems.map((item) => item.isContinuation),
    [false, true, false, false],
  );
  assert.deepEqual(
    messageItems.map((item) => item.isFollowedByContinuation),
    [true, false, false, false],
  );
});

test("buildTimelineItems: empty entries produce no items", () => {
  const { items } = buildTimelineItems([], null);
  assert.equal(items.length, 0);
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

test("buildTimelineDayGroups: moves non-day rows under their day section", () => {
  const entries = [
    entry({ id: "d1a", createdAt: dayAt(2026, 6, 12) }),
    entry({ id: "d1b", createdAt: dayAt(2026, 6, 12, 13) }),
    entry({ id: "d2a", createdAt: dayAt(2026, 6, 13) }),
    entry({ id: "d2b", createdAt: dayAt(2026, 6, 13, 13) }),
  ];
  const { items } = buildTimelineItems(entries, "d2b");

  const groups = buildTimelineDayGroups(items);

  assert.equal(groups.length, 2);
  assert.deepEqual(
    groups.map((group) => group.items.map((item) => item.kind)),
    [
      ["message", "message"],
      ["message", "unread-divider", "message"],
    ],
  );
  assert.ok(groups.every((group) => group.headingTimestamp !== null));
});

test("buildTimelineDayGroups: preserves leading rows without a day divider", () => {
  const leadingRows = [
    { kind: "unread-divider", key: "unread-a" },
    { kind: "message", key: "a", entry: entry({ id: "a" }) },
  ];

  const groups = buildTimelineDayGroups(leadingRows);

  assert.deepEqual(groups, [
    {
      key: "day-undated",
      headingTimestamp: null,
      items: leadingRows,
    },
  ]);
});
