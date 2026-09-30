import assert from "node:assert/strict";
import test from "node:test";

import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import {
  buildTimelineDayGroups,
  buildTimelineItems,
  getTimelineItemKey,
} from "./timelineItems.ts";

function dayAt(year, month, day, hour = 12, minute = 0) {
  return Math.floor(
    new Date(year, month - 1, day, hour, minute, 0).getTime() / 1_000,
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

function memberAddedEntry({ actor = "actor-a", createdAt, id, target }) {
  return entry({
    id,
    createdAt,
    kind: KIND_SYSTEM_MESSAGE,
    body: JSON.stringify({ type: "member_joined", actor, target }),
  });
}

function memberJoinedEntry({ createdAt, id, target }) {
  return memberAddedEntry({ actor: target, createdAt, id, target });
}

function memberLeftEntry({ createdAt, id, target }) {
  return entry({
    id,
    createdAt,
    kind: KIND_SYSTEM_MESSAGE,
    body: JSON.stringify({ type: "member_left", actor: target }),
  });
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

test("buildTimelineItems: contiguous member additions by one actor group", () => {
  const start = dayAt(2026, 6, 14);
  const entries = [
    memberAddedEntry({ id: "a", target: "target-a", createdAt: start }),
    memberAddedEntry({ id: "b", target: "target-b", createdAt: start + 60 }),
    memberAddedEntry({ id: "c", target: "target-c", createdAt: start + 3_600 }),
  ];

  const { items } = buildTimelineItems(entries, null);
  assert.deepEqual(kinds(items), ["day-divider", "system-group"]);
  const group = items.find((item) => item.kind === "system-group");
  assert.deepEqual(
    group?.entries.map((groupEntry) => groupEntry.message.id),
    ["a", "b", "c"],
  );
  assert.equal(group?.key, "c");
});

test("buildTimelineItems: contiguous self-joins group across different members", () => {
  const start = dayAt(2026, 6, 14);
  const entries = [
    memberJoinedEntry({ id: "a", target: "target-a", createdAt: start }),
    memberJoinedEntry({
      id: "b",
      target: "target-b",
      createdAt: start + 60,
    }),
    memberJoinedEntry({
      id: "c",
      target: "target-c",
      createdAt: start + 3_600,
    }),
  ];

  const { items } = buildTimelineItems(entries, null);
  assert.deepEqual(kinds(items), ["day-divider", "system-group"]);
  const group = items.find((item) => item.kind === "system-group");
  assert.deepEqual(
    group?.entries.map((groupEntry) => groupEntry.message.id),
    ["a", "b", "c"],
  );
});

test("buildTimelineItems: mixed arrival actors collapse into one cohort", () => {
  const start = dayAt(2026, 6, 14);
  const entries = [
    memberAddedEntry({
      actor: "viewer",
      createdAt: start,
      id: "elrond",
      target: "elrond",
    }),
    memberAddedEntry({
      actor: "elrond",
      createdAt: start + 60,
      id: "legolas",
      target: "legolas",
    }),
    memberAddedEntry({
      actor: "elrond",
      createdAt: start + 120,
      id: "gimli",
      target: "gimli",
    }),
    memberAddedEntry({
      actor: "viewer",
      createdAt: start + 180,
      id: "gandalf",
      target: "gandalf",
    }),
  ];

  const { items } = buildTimelineItems(entries, null);
  assert.deepEqual(kinds(items), ["day-divider", "system-group"]);
  const group = items.find((item) => item.kind === "system-group");
  assert.deepEqual(
    group?.entries.map((groupEntry) => groupEntry.message.id),
    ["elrond", "legolas", "gimli", "gandalf"],
  );
  assert.equal(group?.key, "gandalf");
});

test("buildTimelineItems: self-joins and additions share one arrival cohort", () => {
  const start = dayAt(2026, 6, 14);
  const entries = [
    memberJoinedEntry({ id: "a", target: "target-a", createdAt: start }),
    memberAddedEntry({
      actor: "target-a",
      id: "b",
      target: "target-b",
      createdAt: start + 60,
    }),
    memberJoinedEntry({
      id: "c",
      target: "target-c",
      createdAt: start + 120,
    }),
  ];

  const { items } = buildTimelineItems(entries, null);
  assert.deepEqual(kinds(items), ["day-divider", "system-group"]);
  const group = items.find((item) => item.kind === "system-group");
  assert.deepEqual(
    group?.entries.map((groupEntry) => groupEntry.message.id),
    ["a", "b", "c"],
  );
});

test("buildTimelineItems: prepending membership history preserves the loaded suffix", () => {
  const start = dayAt(2026, 6, 14);
  const loaded = [
    memberAddedEntry({ id: "b", target: "target-b", createdAt: start + 3_500 }),
    memberAddedEntry({ id: "c", target: "target-c", createdAt: start + 3_601 }),
    entry({ id: "message", createdAt: start + 3_700 }),
  ];
  const prepended = [
    memberAddedEntry({ id: "a", target: "target-a", createdAt: start }),
    ...loaded,
  ];

  const loadedItems = buildTimelineItems(loaded, null).items;
  const prependedItems = buildTimelineItems(prepended, null).items;
  const loadedKeys = loadedItems.slice(1).map((item) => item.key);
  const prependedKeys = prependedItems.slice(1).map((item) => item.key);

  assert.deepEqual(loadedKeys, ["c", "message"]);
  assert.deepEqual(prependedKeys, ["c", "message"]);
  assert.deepEqual(prependedKeys.slice(-loadedKeys.length), loadedKeys);
});

test("buildTimelineItems: contiguous member additions extend a group outside one hour", () => {
  const start = dayAt(2026, 6, 14);
  const entries = [
    memberAddedEntry({ id: "a", target: "target-a", createdAt: start }),
    memberAddedEntry({ id: "b", target: "target-b", createdAt: start + 3_599 }),
    memberAddedEntry({ id: "c", target: "target-c", createdAt: start + 3_601 }),
  ];

  const { items } = buildTimelineItems(entries, null);
  assert.deepEqual(kinds(items), ["day-divider", "system-group"]);
  const group = items.find((item) => item.kind === "system-group");
  assert.deepEqual(
    group?.entries.map((groupEntry) => groupEntry.message.id),
    ["a", "b", "c"],
  );
});

test("buildTimelineItems: arrival cohorts stop at unread dividers", () => {
  const start = dayAt(2026, 6, 14);
  const entries = [
    memberAddedEntry({ id: "a", target: "target-a", createdAt: start }),
    memberAddedEntry({
      actor: "actor-b",
      id: "b",
      target: "target-b",
      createdAt: start + 30,
    }),
    memberJoinedEntry({
      id: "c",
      target: "target-c",
      createdAt: start + 60,
    }),
  ];

  const { items } = buildTimelineItems(entries, "b");
  assert.deepEqual(kinds(items), [
    "day-divider",
    "system",
    "unread-divider",
    "system-group",
  ]);
});

test("buildTimelineItems: arrival cohorts stop at day boundaries", () => {
  const entries = [
    memberAddedEntry({
      actor: "viewer",
      id: "a",
      target: "target-a",
      createdAt: dayAt(2026, 6, 14, 23, 59),
    }),
    memberJoinedEntry({
      id: "b",
      target: "target-b",
      createdAt: dayAt(2026, 6, 15, 0, 0),
    }),
  ];

  const { items } = buildTimelineItems(entries, null);
  assert.deepEqual(kinds(items), [
    "day-divider",
    "system",
    "day-divider",
    "system",
  ]);
});

test("buildTimelineItems: arrival cohorts stop at non-membership rows", () => {
  const start = dayAt(2026, 6, 14);
  const entries = [
    memberAddedEntry({ id: "a", target: "target-a", createdAt: start }),
    entry({ id: "message", createdAt: start + 60 }),
    memberAddedEntry({
      actor: "actor-b",
      id: "b",
      target: "target-c",
      createdAt: start + 90,
    }),
    memberJoinedEntry({ id: "c", target: "target-d", createdAt: start + 120 }),
  ];

  const { items } = buildTimelineItems(entries, null);
  assert.deepEqual(kinds(items), [
    "day-divider",
    "system",
    "message",
    "system-group",
  ]);
});

test("buildTimelineItems: arrival cohorts stop at >1 hour adjacent gaps", () => {
  const start = dayAt(2026, 6, 14);
  const entries = [
    memberAddedEntry({ id: "a", target: "target-a", createdAt: start }),
    memberJoinedEntry({
      id: "b",
      target: "target-b",
      createdAt: start + 3_601,
    }),
  ];

  const { items } = buildTimelineItems(entries, null);
  assert.deepEqual(kinds(items), ["day-divider", "system", "system"]);
});

test("buildTimelineItems: a member joining then leaving is one lifecycle group", () => {
  const start = dayAt(2026, 6, 14);
  const entries = [
    memberJoinedEntry({ id: "joined", target: "member-a", createdAt: start }),
    memberLeftEntry({ id: "left", target: "member-a", createdAt: start + 90 }),
  ];

  const { items } = buildTimelineItems(entries, null);
  assert.deepEqual(kinds(items), ["day-divider", "system-group"]);
  const group = items.find((item) => item.kind === "system-group");
  assert.deepEqual(
    group?.entries.map((groupEntry) => groupEntry.message.id),
    ["joined", "left"],
  );
});

test("buildTimelineItems: duplicate self-joins then leaving stay one group keyed by the departure", () => {
  const start = dayAt(2026, 6, 14);
  const entries = [
    memberJoinedEntry({ id: "joined-1", target: "member-a", createdAt: start }),
    memberJoinedEntry({
      id: "joined-2",
      target: "member-a",
      createdAt: start + 30,
    }),
    memberLeftEntry({ id: "left", target: "member-a", createdAt: start + 90 }),
  ];

  const { items } = buildTimelineItems(entries, null);
  assert.deepEqual(kinds(items), ["day-divider", "system-group"]);
  const group = items.find((item) => item.kind === "system-group");
  assert.deepEqual(
    group?.entries.map((groupEntry) => groupEntry.message.id),
    ["joined-1", "joined-2", "left"],
  );
  // Anchored on the newest entry so prepending older history cannot repartition
  // the loaded rows; splitting this into separate rows would change both.
  assert.equal(group?.key, "left");
});

test("buildTimelineItems: consecutive same-author messages within the window are grouped", () => {
  const entries = [
    entry({ id: "a", pubkey: "author-a", createdAt: dayAt(2026, 6, 14) }),
    entry({
      id: "b",
      pubkey: "AUTHOR-A",
      createdAt: dayAt(2026, 6, 14, 12, 2),
    }),
    entry({
      id: "c",
      pubkey: "author-b",
      createdAt: dayAt(2026, 6, 14, 12, 3),
    }),
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

test("buildTimelineItems: pending messages remain standalone until acknowledged", () => {
  const entries = [
    entry({ id: "a", pubkey: "author-a", createdAt: dayAt(2026, 6, 14) }),
    entry({
      id: "b",
      pubkey: "author-a",
      createdAt: dayAt(2026, 6, 14, 12, 2),
      pending: true,
    }),
    entry({
      id: "c",
      pubkey: "author-a",
      createdAt: dayAt(2026, 6, 14, 12, 3),
    }),
  ];

  const messageItems = buildTimelineItems(entries, null).items.filter(
    (item) => item.kind === "message",
  );

  assert.deepEqual(
    messageItems.map((item) => item.isContinuation),
    [false, false, false],
  );
  assert.deepEqual(
    messageItems.map((item) => item.isFollowedByContinuation),
    [false, false, false],
  );
});

test("buildTimelineItems: sent-from-thread messages start a fresh author group", () => {
  const entries = [
    entry({ id: "a", pubkey: "author-a", createdAt: dayAt(2026, 6, 14) }),
    entry({
      id: "b",
      pubkey: "author-a",
      createdAt: dayAt(2026, 6, 14, 12, 2),
      tags: [["buzz:sent-from-thread", "root-event", "Root summary"]],
    }),
    entry({
      id: "c",
      pubkey: "author-a",
      createdAt: dayAt(2026, 6, 14, 12, 3),
    }),
  ];

  const messageItems = buildTimelineItems(entries, null).items.filter(
    (item) => item.kind === "message",
  );

  assert.deepEqual(
    messageItems.map((item) => item.isContinuation),
    [false, false, true],
  );
  assert.deepEqual(
    messageItems.map((item) => item.isFollowedByContinuation),
    [false, true, false],
  );
});

test("buildTimelineItems: same-author messages past the window start a new group", () => {
  const author = "author-a";
  const entries = [
    entry({ id: "a", pubkey: author, createdAt: dayAt(2026, 6, 14, 12, 0) }),
    // 8 min later — within the 10-min window, groups as a continuation.
    entry({ id: "b", pubkey: author, createdAt: dayAt(2026, 6, 14, 12, 8) }),
    // 12 min after "b" — past the window, breaks into a new thought.
    entry({ id: "c", pubkey: author, createdAt: dayAt(2026, 6, 14, 12, 20) }),
    // 5 min after "c" — within the window again, groups onto "c".
    entry({ id: "d", pubkey: author, createdAt: dayAt(2026, 6, 14, 12, 25) }),
  ];

  const messageItems = buildTimelineItems(entries, null).items.filter(
    (item) => item.kind === "message",
  );

  assert.deepEqual(
    messageItems.map((item) => item.isContinuation),
    [false, true, false, true],
  );
  assert.deepEqual(
    messageItems.map((item) => item.isFollowedByContinuation),
    [true, false, true, false],
  );
});

test("buildTimelineItems: window is measured against the previous message, not the group start", () => {
  const author = "author-a";
  // Each message is 8 min after the one above it — a steady stream that never
  // gaps out, so grouping continues even though the span (16 min) exceeds the
  // 10-min window.
  const entries = [
    entry({ id: "a", pubkey: author, createdAt: dayAt(2026, 6, 14, 12, 0) }),
    entry({ id: "b", pubkey: author, createdAt: dayAt(2026, 6, 14, 12, 8) }),
    entry({ id: "c", pubkey: author, createdAt: dayAt(2026, 6, 14, 12, 16) }),
  ];

  const messageItems = buildTimelineItems(entries, null).items.filter(
    (item) => item.kind === "message",
  );

  assert.deepEqual(
    messageItems.map((item) => item.isContinuation),
    [false, true, true],
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
        createdAt: dayAt(2026, 6, 14, 12, 1),
      }),
      summary: {
        threadHeadId: "b",
        replyCount: 1,
        lastReplyAt: dayAt(2026, 6, 14, 12, 1),
        participants: [],
      },
    },
    entry({
      id: "c",
      pubkey: sameAuthor,
      createdAt: dayAt(2026, 6, 14, 12, 2),
    }),
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
