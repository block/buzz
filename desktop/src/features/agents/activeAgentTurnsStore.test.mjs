import assert from "node:assert/strict";
import { describe, it, beforeEach } from "node:test";

import {
  syncAgentTurnsFromEvents,
  getActiveChannelsForAgent,
  resetActiveAgentTurnsStore,
  subscribeActiveAgentTurns,
} from "./activeAgentTurnsStore.ts";

const AGENT =
  "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";

function makeEvent(overrides) {
  return {
    seq: 1,
    timestamp: "2024-01-01T00:00:00Z",
    kind: "turn_started",
    agentIndex: 0,
    channelId: "chan-1",
    sessionId: "sess-1",
    turnId: "turn-1",
    payload: null,
    ...overrides,
  };
}

describe("activeAgentTurnsStore", () => {
  beforeEach(() => {
    resetActiveAgentTurnsStore();
  });

  describe("seq filtering", () => {
    it("processes events with increasing seq", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
      ]);
      const channels = getActiveChannelsForAgent(AGENT);
      assert.equal(channels.size, 1);
      assert.ok(channels.has("c1"));
    });

    it("skips events with seq <= lastProcessedSeq", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 5, turnId: "t1", channelId: "c1" }),
      ]);
      // Try to process an older event — should be ignored
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 3, turnId: "t2", channelId: "c2" }),
      ]);
      const channels = getActiveChannelsForAgent(AGENT);
      assert.equal(channels.size, 1);
      assert.ok(channels.has("c1"));
      assert.ok(!channels.has("c2"));
    });

    it("skips duplicate seq", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
      ]);
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t2", channelId: "c2" }),
      ]);
      const channels = getActiveChannelsForAgent(AGENT);
      assert.equal(channels.size, 1);
      assert.ok(channels.has("c1"));
    });
  });

  describe("seq restart detection", () => {
    it("resets lastProcessedSeq when seq regresses (agent restart)", () => {
      // Process events up to seq 50
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 50, turnId: "t1", channelId: "c1" }),
      ]);
      assert.equal(getActiveChannelsForAgent(AGENT).size, 1);

      // Agent restarts — seq goes back to 1. This should be accepted.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t2", channelId: "c2" }),
      ]);
      const channels = getActiveChannelsForAgent(AGENT);
      assert.ok(channels.has("c2"), "post-restart event should be processed");
    });

    it("processes subsequent events after restart detection", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 100, turnId: "t1", channelId: "c1" }),
      ]);

      // Restart: seq goes to 1, then 2, then 3
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t2", channelId: "c2" }),
        makeEvent({ seq: 2, turnId: "t3", channelId: "c3" }),
        makeEvent({
          seq: 3,
          kind: "turn_completed",
          turnId: "t2",
          channelId: "c2",
        }),
      ]);
      const channels = getActiveChannelsForAgent(AGENT);
      // t1 still active (not ended), t2 ended, t3 still active
      assert.ok(channels.has("c1"));
      assert.ok(!channels.has("c2"));
      assert.ok(channels.has("c3"));
    });
  });

  describe("eviction at MAX_TURNS_PER_AGENT", () => {
    it("evicts oldest turn when exceeding 4 concurrent turns", () => {
      const events = [];
      for (let i = 1; i <= 5; i++) {
        events.push(
          makeEvent({
            seq: i,
            turnId: `t${i}`,
            channelId: `c${i}`,
            timestamp: `2024-01-01T00:0${i}:00Z`,
          }),
        );
      }
      syncAgentTurnsFromEvents(AGENT, events);
      const channels = getActiveChannelsForAgent(AGENT);
      // Should have evicted c1 (oldest) to make room for c5
      assert.equal(channels.size, 4);
      assert.ok(!channels.has("c1"), "oldest turn should be evicted");
      assert.ok(channels.has("c2"));
      assert.ok(channels.has("c5"));
    });
  });

  describe("endTurn turnId-vs-channelId fallback", () => {
    it("ends turn by turnId when provided", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
        makeEvent({
          seq: 2,
          kind: "turn_completed",
          turnId: "t1",
          channelId: null,
        }),
      ]);
      assert.equal(getActiveChannelsForAgent(AGENT).size, 0);
    });

    it("falls back to channelId when turnId is null", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
        makeEvent({
          seq: 2,
          kind: "turn_completed",
          turnId: null,
          channelId: "c1",
        }),
      ]);
      assert.equal(getActiveChannelsForAgent(AGENT).size, 0);
    });

    it("does nothing when both turnId and channelId are null", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
        makeEvent({
          seq: 2,
          kind: "turn_completed",
          turnId: null,
          channelId: null,
        }),
      ]);
      // Turn should still be active — no way to identify which to end
      assert.equal(getActiveChannelsForAgent(AGENT).size, 1);
    });

    it("channelId fallback removes only one matching turn", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
        makeEvent({ seq: 2, turnId: "t2", channelId: "c1" }),
        makeEvent({
          seq: 3,
          kind: "turn_completed",
          turnId: null,
          channelId: "c1",
        }),
      ]);
      // Only one of the two turns in c1 should be removed
      const channels = getActiveChannelsForAgent(AGENT);
      assert.equal(channels.size, 1);
      assert.ok(channels.has("c1"));
    });
  });

  describe("listener notifications", () => {
    it("notifies on turn_started", () => {
      let called = 0;
      const unsub = subscribeActiveAgentTurns(() => {
        called++;
      });
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
      ]);
      assert.ok(called > 0);
      unsub();
    });

    it("notifies on turn_completed", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
      ]);
      let called = 0;
      const unsub = subscribeActiveAgentTurns(() => {
        called++;
      });
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 2, kind: "turn_completed", turnId: "t1" }),
      ]);
      assert.ok(called > 0);
      unsub();
    });
  });

  describe("getActiveChannelsForAgent", () => {
    it("returns EMPTY_SET for null/undefined pubkey", () => {
      assert.equal(getActiveChannelsForAgent(null).size, 0);
      assert.equal(getActiveChannelsForAgent(undefined).size, 0);
    });

    it("returns stable reference when unchanged", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
      ]);
      const ref1 = getActiveChannelsForAgent(AGENT);
      const ref2 = getActiveChannelsForAgent(AGENT);
      assert.equal(ref1, ref2, "should return cached reference");
    });
  });
});
