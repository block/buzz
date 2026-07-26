import assert from "node:assert/strict";
import test from "node:test";

import {
  isInjectedTranscriptId,
  mergeTranscriptItems,
} from "./agentSessionPanelLayout.ts";

const base = [
  { id: "a", timestamp: "2026-07-26T10:00:00.000Z" },
  { id: "b", timestamp: "2026-07-26T12:00:00.000Z" },
];

test("returns base untouched when there is nothing to merge", () => {
  assert.deepEqual(mergeTranscriptItems(base, []), base);
});

test("interleaves extra rows by timestamp", () => {
  const merged = mergeTranscriptItems(base, [
    { id: "mid", timestamp: "2026-07-26T11:00:00.000Z" },
  ]);
  assert.deepEqual(
    merged.map((i) => i.id),
    ["a", "mid", "b"],
  );
});

test("appends extra rows that are newer than everything", () => {
  const merged = mergeTranscriptItems(base, [
    { id: "late", timestamp: "2026-07-26T23:00:00.000Z" },
  ]);
  assert.deepEqual(
    merged.map((i) => i.id),
    ["a", "b", "late"],
  );
});

test("keeps base before extra on identical timestamps", () => {
  const merged = mergeTranscriptItems(base, [
    { id: "tie", timestamp: "2026-07-26T12:00:00.000Z" },
  ]);
  assert.deepEqual(
    merged.map((i) => i.id),
    ["a", "b", "tie"],
  );
});

test("drops extra rows whose id already exists in base", () => {
  const merged = mergeTranscriptItems(base, [
    { id: "a", timestamp: "2026-07-26T09:00:00.000Z" },
  ]);
  assert.deepEqual(
    merged.map((i) => i.id),
    ["a", "b"],
  );
});

test("sorts unparseable timestamps last instead of throwing", () => {
  const merged = mergeTranscriptItems(base, [
    { id: "bad", timestamp: "nonsense" },
  ]);
  assert.deepEqual(
    merged.map((i) => i.id),
    ["a", "b", "bad"],
  );
});

test("drops an extra row whose source messageId is already rendered", () => {
  const merged = mergeTranscriptItems(
    [
      {
        id: "user:c:evt1",
        timestamp: "2026-07-26T11:00:00.000Z",
        messageId: "evt1",
      },
    ],
    [
      {
        id: "prompt:evt1",
        timestamp: "2026-07-26T11:00:00.000Z",
        messageId: "evt1",
      },
    ],
  );
  assert.deepEqual(
    merged.map((i) => i.id),
    ["user:c:evt1"],
  );
});

test("keeps an extra row when its messageId is not yet rendered", () => {
  const merged = mergeTranscriptItems(
    [
      {
        id: "user:c:evt1",
        timestamp: "2026-07-26T11:00:00.000Z",
        messageId: "evt1",
      },
    ],
    [
      {
        id: "prompt:evt2",
        timestamp: "2026-07-26T11:05:00.000Z",
        messageId: "evt2",
      },
    ],
  );
  assert.deepEqual(
    merged.map((i) => i.id),
    ["user:c:evt1", "prompt:evt2"],
  );
});

test("recognises both injected row prefixes", () => {
  assert.equal(isInjectedTranscriptId("reply:abc"), true);
  assert.equal(isInjectedTranscriptId("prompt:abc"), true);
  assert.equal(isInjectedTranscriptId("user:chan:abc"), false);
  assert.equal(isInjectedTranscriptId("assistant:chan:abc"), false);
});
