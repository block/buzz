import assert from "node:assert/strict";
import test from "node:test";

import {
  buildVirtualTimelineRows,
  findVirtualRowIndexForMessage,
} from "./buildVirtualTimelineRows.ts";

const DAY = 24 * 60 * 60;
// Anchor at local noon so day boundaries are unambiguous across timezones.
const DAY_1 = Math.floor(new Date(2026, 0, 1, 12, 0, 0).getTime() / 1000);
const DAY_2 = DAY_1 + DAY;
const DAY_3 = DAY_2 + DAY;

function message(overrides) {
  return {
    id: "message",
    createdAt: DAY_1,
    depth: 0,
    kind: 9,
    ...overrides,
  };
}

test("empty snapshot produces no rows", () => {
  assert.deepEqual(buildVirtualTimelineRows([]), []);
});

test("empty snapshot with includeIntro emits only the intro row", () => {
  const rows = buildVirtualTimelineRows([], { includeIntro: true });
  assert.equal(rows.length, 1);
  assert.equal(rows[0].kind, "intro");
  assert.equal(rows[0].key, "channel-intro");
});

test("includeIntro prepends the intro row before dividers and messages", () => {
  const rows = buildVirtualTimelineRows([message({ id: "a" })], {
    includeIntro: true,
  });
  assert.deepEqual(
    rows.map((row) => row.kind),
    ["intro", "day-divider", "message"],
  );
});

test("findVirtualRowIndexForMessage still resolves after an intro row is prepended", () => {
  const messages = [
    message({ id: "a", createdAt: DAY_1 }),
    message({ id: "b", createdAt: DAY_2 }),
  ];
  const rows = buildVirtualTimelineRows(messages, { includeIntro: true });
  // layout: [intro, div, a, div, b]
  assert.equal(findVirtualRowIndexForMessage(rows, "a", messages), 2);
  assert.equal(findVirtualRowIndexForMessage(rows, "b", messages), 4);
});

test("single message emits one divider then the message row", () => {
  const rows = buildVirtualTimelineRows([message({ id: "a" })]);
  assert.equal(rows.length, 2);
  assert.equal(rows[0].kind, "day-divider");
  assert.equal(rows[0].messageIndex, -1);
  assert.equal(rows[1].kind, "message");
  assert.equal(rows[1].messageIndex, 0);
  assert.equal(rows[1].key, "a");
});

test("messages on the same day share a single divider", () => {
  const rows = buildVirtualTimelineRows([
    message({ id: "a", createdAt: DAY_1 }),
    message({ id: "b", createdAt: DAY_1 + 60 }),
    message({ id: "c", createdAt: DAY_1 + 120 }),
  ]);
  // 1 divider + 3 messages
  assert.equal(rows.length, 4);
  assert.equal(rows.filter((r) => r.kind === "day-divider").length, 1);
  assert.deepEqual(
    rows.filter((r) => r.kind === "message").map((r) => r.messageIndex),
    [0, 1, 2],
  );
});

test("a new calendar day inserts a fresh divider before its first message", () => {
  const rows = buildVirtualTimelineRows([
    message({ id: "a", createdAt: DAY_1 }),
    message({ id: "b", createdAt: DAY_2 }),
    message({ id: "c", createdAt: DAY_3 }),
  ]);
  // 3 dividers + 3 messages, strictly interleaved
  assert.deepEqual(
    rows.map((r) => r.kind),
    [
      "day-divider",
      "message",
      "day-divider",
      "message",
      "day-divider",
      "message",
    ],
  );
  // message indices stay monotonic and correct across dividers
  assert.deepEqual(
    rows.filter((r) => r.kind === "message").map((r) => r.messageIndex),
    [0, 1, 2],
  );
});

test("message row key prefers renderKey over id (optimistic-send stability)", () => {
  const rows = buildVirtualTimelineRows([
    message({ id: "server-id", renderKey: "local-key" }),
  ]);
  const messageRow = rows.find((r) => r.kind === "message");
  assert.equal(messageRow.key, "local-key");
});

test("divider keys are stable across re-flatten of the same snapshot", () => {
  const snapshot = [
    message({ id: "a", createdAt: DAY_1 }),
    message({ id: "b", createdAt: DAY_2 }),
  ];
  const first = buildVirtualTimelineRows(snapshot);
  const second = buildVirtualTimelineRows(snapshot);
  assert.deepEqual(
    first.map((r) => r.key),
    second.map((r) => r.key),
  );
});

test("findVirtualRowIndexForMessage returns the FLAT index, accounting for dividers", () => {
  const messages = [
    message({ id: "a", createdAt: DAY_1 }),
    message({ id: "b", createdAt: DAY_2 }),
    message({ id: "c", createdAt: DAY_2 + 60 }),
  ];
  const rows = buildVirtualTimelineRows(messages);
  // layout: [div, a, div, b, c] -> flat indices 0..4
  assert.equal(findVirtualRowIndexForMessage(rows, "a", messages), 1);
  assert.equal(findVirtualRowIndexForMessage(rows, "b", messages), 3);
  assert.equal(findVirtualRowIndexForMessage(rows, "c", messages), 4);
});

test("findVirtualRowIndexForMessage returns -1 for an absent or empty target", () => {
  const messages = [message({ id: "a" })];
  const rows = buildVirtualTimelineRows(messages);
  assert.equal(findVirtualRowIndexForMessage(rows, "missing", messages), -1);
  assert.equal(findVirtualRowIndexForMessage(rows, null, messages), -1);
  assert.equal(findVirtualRowIndexForMessage(rows, undefined, messages), -1);
});

test("prepend keeps surviving rows' keys stable (native position-retention contract)", () => {
  // Older page prepended at the top: 'a' was the head, now 'older' precedes it.
  const before = [message({ id: "a", createdAt: DAY_2 })];
  const after = [
    message({ id: "older", createdAt: DAY_1 }),
    message({ id: "a", createdAt: DAY_2 }),
  ];
  const rowsBefore = buildVirtualTimelineRows(before);
  const rowsAfter = buildVirtualTimelineRows(after);

  const keyOfA = (rows) =>
    rows.find((r) => r.kind === "message" && r.key === "a")?.key;
  // 'a' keeps its identity across the prepend -> virtualizer measurement cache
  // survives -> scroll position re-anchors natively (no double-rAF correction).
  assert.equal(keyOfA(rowsBefore), "a");
  assert.equal(keyOfA(rowsAfter), "a");
});
