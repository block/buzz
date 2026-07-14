import assert from "node:assert/strict";
import test from "node:test";

import {
  buildTranscriptDisplayBlocks,
  getDisplayBlockKey,
} from "@/features/agents/ui/agentSessionTranscriptGrouping.ts";
import { observerEventScrollId } from "@/features/agents/ui/agentSessionPanelLayout.ts";

// ─── Helpers ────────────────────────────────────────────────────────────────────

/**
 * Build a minimal TranscriptItem (tool-call shape) stamped with a given session
 * and turn. Reuses the pattern from agentSessionTranscriptGrouping.test.mjs.
 */
function mkItem(id, sessionId, turnId, ts = "2026-07-08T00:00:00.000Z") {
  return {
    id,
    type: "tool",
    renderClass: "generic",
    descriptor: {
      renderClass: "generic",
      label: id,
      preview: id,
      source: "harness",
      groupKey: id,
    },
    title: id,
    toolName: id,
    buzzToolName: null,
    status: "completed",
    args: {},
    result: "",
    isError: false,
    timestamp: ts,
    startedAt: ts,
    completedAt: ts,
    turnId,
    sessionId,
    channelId: "chan-1",
  };
}

/** Build a minimal ObserverEvent for raw-mode id derivation. */
function mkEvent(seq, ts = "2026-07-08T00:00:00.000Z") {
  return { seq, timestamp: ts };
}

/**
 * Derive transcript block ids the same way AgentSessionThreadPanel does:
 *   items → buildTranscriptDisplayBlocks → getDisplayBlockKey
 */
function deriveBlockIds(items) {
  const blocks = buildTranscriptDisplayBlocks(items);
  return blocks.map(getDisplayBlockKey);
}

// ── Test group 1: raw events append, block ids unchanged, at-bottom ─────────
//
// When raw events arrive that do NOT produce new display blocks (e.g. streaming
// tool updates within an existing turn), the derived block id sequence must be
// identical. The useStableArrayShallow hook preserves the prior reference when
// the string[] is shallow-equal, preventing the restoration effect from firing.

test("deriveBlockIds_sameBlockIds_whenSameTurnItemAppended", () => {
  // Turn-1 has one item: produces one turn block.
  const items1 = [mkItem("tool-1", "sess-1", "turn-1")];
  const ids1 = deriveBlockIds(items1);

  // A second item on the SAME turn — no new block, block key unchanged.
  const items2 = [...items1, mkItem("tool-2", "sess-1", "turn-1")];
  const ids2 = deriveBlockIds(items2);

  assert.deepEqual(
    ids1,
    ids2,
    "appending a same-turn item must not change the block id sequence",
  );
});

test("deriveBlockIds_sameBlockIds_whenMultipleSameTurnItemsAppended", () => {
  // Simulate a streaming turn: 5 items on the same turn.
  const items5 = Array.from({ length: 5 }, (_, i) =>
    mkItem(`tool-${i + 1}`, "sess-1", "turn-1"),
  );
  const ids5 = deriveBlockIds(items5);

  // Add 5 more items on the same turn.
  const items10 = [
    ...items5,
    ...Array.from({ length: 5 }, (_, i) =>
      mkItem(`tool-${i + 6}`, "sess-1", "turn-1"),
    ),
  ];
  const ids10 = deriveBlockIds(items10);

  assert.deepEqual(
    ids5,
    ids10,
    "same-turn streaming items must not change block ids (mid-history no-yank invariant)",
  );
});

// ── Test group 2: mid-history, raw events grow, block ids unchanged ─────────

test("deriveBlockIds_midHistory_multiTurn_stableKeys", () => {
  // Two turns already exist.
  const items1 = [
    mkItem("tool-a", "sess-1", "turn-1"),
    mkItem("tool-b", "sess-1", "turn-2"),
  ];
  const ids1 = deriveBlockIds(items1);

  // Append a new item to an existing turn — keys stay the same.
  const items2 = [...items1, mkItem("tool-c", "sess-1", "turn-2")];
  const ids2 = deriveBlockIds(items2);

  assert.deepEqual(
    ids1,
    ids2,
    "appending to an existing turn must not change the id sequence",
  );
});

// ── Test group 3: new display block — id sequence grows ─────────────────────

test("deriveBlockIds_newSession_addsBlockIds", () => {
  const items1 = [
    mkItem("tool-a", "sess-1", "turn-1", "2026-07-08T00:00:01.000Z"),
  ];
  const ids1 = deriveBlockIds(items1);

  // New session → new turn block + boundary block.
  const items2 = [
    ...items1,
    mkItem("tool-b", "sess-2", "turn-2", "2026-07-08T00:00:02.000Z"),
  ];
  const ids2 = deriveBlockIds(items2);

  assert.ok(
    ids2.length > ids1.length,
    "a new session must produce additional block ids",
  );
  // Original ids must still be present.
  for (const id of ids1) {
    assert.ok(ids2.includes(id), `original id "${id}" must still be present`);
  }
});

test("deriveBlockIds_newTurn_addsBlockId", () => {
  const items1 = [mkItem("tool-a", "sess-1", "turn-1")];
  const ids1 = deriveBlockIds(items1);

  // New turn in the same session → new turn block.
  const items2 = [...items1, mkItem("tool-b", "sess-1", "turn-2")];
  const ids2 = deriveBlockIds(items2);

  assert.equal(
    ids2.length,
    ids1.length + 1,
    "new turn adds exactly one block id",
  );
  assert.ok(ids2.includes("turn:turn-2"), "new turn block key must be present");
});

// ── Test group 4: key-parity invariant ──────────────────────────────────────

test("deriveBlockIds_deterministic_sameInputSameOutput", () => {
  const items = [
    mkItem("tool-a", "sess-1", "turn-1", "2026-07-08T00:00:01.000Z"),
    mkItem("tool-b", "sess-1", "turn-1", "2026-07-08T00:00:02.000Z"),
    mkItem("tool-c", "sess-2", "turn-2", "2026-07-08T00:00:03.000Z"),
  ];

  const ids1 = deriveBlockIds(items);
  const ids2 = deriveBlockIds(items);

  assert.deepEqual(ids1, ids2, "same items must produce identical block ids");
});

test("deriveBlockIds_transientReorder_keysStable", () => {
  // The first-turn sequence can produce a transient [turn, single] → [single, turn]
  // reorder when session_resolved arrives. Key identities must be stable.
  const ts = "2026-07-08T10:00:00.000Z";

  // Partial: turn_started + session/new — before session_resolved.
  const partialItems = [
    {
      id: "turn-started",
      type: "lifecycle",
      renderClass: "lifecycle",
      title: "Turn started",
      text: "",
      timestamp: ts,
      acpSource: "turn_started",
      turnId: "turn-001",
      sessionId: null,
      channelId: "chan-1",
    },
    {
      id: "system-prompt:chan-1",
      type: "metadata",
      renderClass: "raw-rail",
      title: "System prompt",
      sections: [{ title: "Base", body: "You are a helpful AI assistant." }],
      timestamp: ts,
      acpSource: "session/new",
      turnId: null,
      sessionId: null,
      channelId: "chan-1",
    },
  ];

  const partialIds = new Set(deriveBlockIds(partialItems));

  // Full: add session_resolved — may reorder blocks.
  const fullItems = [
    ...partialItems,
    {
      id: "session-resolved",
      type: "lifecycle",
      renderClass: "lifecycle",
      title: "Session ready",
      text: "",
      timestamp: ts,
      acpSource: "session_resolved",
      turnId: "turn-001",
      sessionId: "session-001",
      channelId: "chan-1",
    },
  ];

  const fullIds = new Set(deriveBlockIds(fullItems));

  assert.deepEqual(
    partialIds,
    fullIds,
    "block key identities must be identical before and after session_resolved (order may differ)",
  );
});

// ── Test group 5: mode toggle reset key ─────────────────────────────────────

test("modeToggle_resetKeyDiffers_betweenRawAndTranscript", () => {
  const rawFeedScopeKey = "agent-pk:chan-1";
  const rawKey = `${rawFeedScopeKey}:raw`;
  const transcriptKey = `${rawFeedScopeKey}:transcript`;

  assert.notEqual(
    rawKey,
    transcriptKey,
    "raw and transcript reset keys must differ to force hook re-init on toggle",
  );
});

test("modeToggle_rawIds_matchObserverEventScrollId", () => {
  const events = [
    mkEvent(1, "2026-07-08T00:00:01.000Z"),
    mkEvent(2, "2026-07-08T00:00:02.000Z"),
  ];
  const rawIds = events.map((e) => observerEventScrollId(e));

  assert.equal(rawIds.length, events.length, "one raw id per event");
  assert.equal(
    new Set(rawIds).size,
    rawIds.length,
    "all raw ids must be unique",
  );
});

test("modeToggle_transcriptIds_disjointFromRawIds", () => {
  // Transcript block ids (turn:xxx, session-boundary:xxx, item-id) live in a
  // different namespace from raw ids (seq:timestamp). They must never collide.
  const events = [
    mkEvent(1, "2026-07-08T00:00:01.000Z"),
    mkEvent(2, "2026-07-08T00:00:02.000Z"),
  ];
  const rawIds = new Set(events.map((e) => observerEventScrollId(e)));

  const items = [
    mkItem("tool-a", "sess-1", "turn-1", "2026-07-08T00:00:01.000Z"),
    mkItem("tool-b", "sess-2", "turn-2", "2026-07-08T00:00:02.000Z"),
  ];
  const blockIds = deriveBlockIds(items);

  for (const blockId of blockIds) {
    assert.ok(
      !rawIds.has(blockId),
      `block id "${blockId}" must not collide with any raw id`,
    );
  }
});
