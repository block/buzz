import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  deriveActivityTurns,
  eventsForTurn,
  summarizeTurnActivity,
} from "./activityTurns.ts";

const BASE_T = Date.parse("2026-08-17T12:00:00.000Z");

function ts(offsetMs = 0) {
  return new Date(BASE_T + offsetMs).toISOString();
}

function frame(overrides = {}) {
  return {
    seq: 1,
    timestamp: ts(0),
    kind: "acp_read",
    agentIndex: 0,
    channelId: "chan-1",
    sessionId: null,
    turnId: "turn-1",
    payload: null,
    ...overrides,
  };
}

function turnStarted(seq, offsetMs, turnId, payload = {}) {
  return frame({
    seq,
    timestamp: ts(offsetMs),
    kind: "turn_started",
    turnId,
    payload: { source: "channel", triggeringEventIds: [], ...payload },
  });
}

function turnCompleted(seq, offsetMs, turnId) {
  return frame({
    seq,
    timestamp: ts(offsetMs),
    kind: "turn_completed",
    turnId,
    payload: {},
  });
}

function activityEvent(overrides = {}) {
  return {
    agentId: "agent-1",
    t: BASE_T,
    kind: "r",
    path: "src/a.ts",
    ...overrides,
  };
}

describe("deriveActivityTurns", () => {
  it("bounds a turn by turn_started and turn_completed", () => {
    const turns = deriveActivityTurns([
      turnStarted(1, 0, "turn-1", {
        triggeringEventIds: ["evt-a", "evt-b"],
      }),
      frame({ seq: 2, timestamp: ts(1_000), turnId: "turn-1" }),
      turnCompleted(3, 5_000, "turn-1"),
    ]);

    assert.equal(turns.length, 1);
    assert.equal(turns[0].id, "turn-1");
    assert.equal(turns[0].startT, BASE_T);
    assert.equal(turns[0].endT, BASE_T + 5_000);
    assert.equal(turns[0].source, "channel");
    assert.deepEqual(turns[0].triggeringEventIds, ["evt-a", "evt-b"]);
  });

  it("keeps the newest turn open when it has no turn_completed", () => {
    const turns = deriveActivityTurns([
      turnStarted(1, 0, "turn-1"),
      turnCompleted(2, 2_000, "turn-1"),
      turnStarted(3, 10_000, "turn-2"),
      frame({ seq: 4, timestamp: ts(11_000), turnId: "turn-2" }),
    ]);

    assert.equal(turns.length, 2);
    assert.equal(turns[0].endT, BASE_T + 2_000);
    assert.equal(turns[1].endT, undefined);
  });

  it("closes abandoned non-newest turns at their last seen frame", () => {
    const turns = deriveActivityTurns([
      turnStarted(1, 0, "turn-1"),
      frame({ seq: 2, timestamp: ts(3_000), turnId: "turn-1" }),
      turnStarted(3, 10_000, "turn-2"),
    ]);

    assert.equal(turns[0].endT, BASE_T + 3_000);
    assert.equal(turns[1].endT, undefined);
  });

  it("adopts a turn from mid-stream frames when turn_started was missed", () => {
    const turns = deriveActivityTurns([
      frame({ seq: 5, timestamp: ts(4_000), turnId: "turn-x" }),
      turnCompleted(6, 6_000, "turn-x"),
    ]);

    assert.equal(turns.length, 1);
    assert.equal(turns[0].startT, BASE_T + 4_000);
    assert.equal(turns[0].endT, BASE_T + 6_000);
    assert.deepEqual(turns[0].triggeringEventIds, []);
  });

  it("records the session id from session_resolved", () => {
    const turns = deriveActivityTurns([
      turnStarted(1, 0, "turn-1"),
      frame({
        seq: 2,
        timestamp: ts(500),
        kind: "session_resolved",
        turnId: "turn-1",
        payload: { sessionId: "sess-42", isNewSession: true },
      }),
      turnCompleted(3, 1_000, "turn-1"),
    ]);

    assert.equal(turns[0].sessionId, "sess-42");
  });

  it("marks heartbeat turns", () => {
    const turns = deriveActivityTurns([
      turnStarted(1, 0, "turn-hb", {
        source: "heartbeat",
        triggeringEventIds: [],
      }),
      turnCompleted(2, 1_000, "turn-hb"),
    ]);

    assert.equal(turns[0].source, "heartbeat");
  });

  it("ignores frames without a turnId", () => {
    const turns = deriveActivityTurns([
      frame({ seq: 1, turnId: null, kind: "agent_initialized" }),
    ]);

    assert.equal(turns.length, 0);
  });
});

describe("eventsForTurn", () => {
  it("filters events by sourceTurnId", () => {
    const events = [
      activityEvent({ sourceTurnId: "turn-1", path: "a.ts" }),
      activityEvent({ sourceTurnId: "turn-2", path: "b.ts" }),
      activityEvent({ sourceTurnId: "turn-1", path: "c.ts" }),
      activityEvent({ path: "d.ts" }),
    ];

    const filtered = eventsForTurn(events, "turn-1");
    assert.deepEqual(
      filtered.map((event) => event.path),
      ["a.ts", "c.ts"],
    );
  });
});

describe("summarizeTurnActivity", () => {
  it("counts distinct files, reads, and edits", () => {
    const summary = summarizeTurnActivity([
      activityEvent({ kind: "r", path: "a.ts" }),
      activityEvent({ kind: "w", path: "a.ts" }),
      activityEvent({ kind: "c", path: "b.ts" }),
      activityEvent({ kind: "r", path: "c.ts" }),
    ]);

    assert.deepEqual(summary, { fileCount: 3, readCount: 2, editCount: 2 });
  });

  it("excludes pathless status events", () => {
    const summary = summarizeTurnActivity([
      activityEvent({ kind: "s", path: undefined }),
      activityEvent({ kind: "r", path: "a.ts" }),
    ]);

    assert.deepEqual(summary, { fileCount: 1, readCount: 1, editCount: 0 });
  });

  it("returns zeros for no events", () => {
    assert.deepEqual(summarizeTurnActivity([]), {
      fileCount: 0,
      readCount: 0,
      editCount: 0,
    });
  });
});
