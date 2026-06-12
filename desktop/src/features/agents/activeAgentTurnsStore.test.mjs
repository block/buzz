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

    it("skips events at or below the watermark", () => {
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
    it("processes post-restart events whose timestamp climbs past the watermark", () => {
      // Process events up to seq 50.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 50,
          turnId: "t1",
          channelId: "c1",
          timestamp: "2024-01-01T00:00:00Z",
        }),
      ]);
      assert.equal(getActiveChannelsForAgent(AGENT).size, 1);

      // Agent restarts — seq resets to 1, but wall-clock timestamp keeps
      // climbing. The composite watermark accepts it on timestamp alone.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 1,
          turnId: "t2",
          channelId: "c2",
          timestamp: "2024-01-01T00:01:00Z",
        }),
      ]);
      const channels = getActiveChannelsForAgent(AGENT);
      assert.ok(channels.has("c2"), "post-restart event should be processed");
    });

    it("processes subsequent events after restart", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 100,
          turnId: "t1",
          channelId: "c1",
          timestamp: "2024-01-01T00:00:00Z",
        }),
      ]);

      // Restart: seq goes 1, 2, 3 with climbing timestamps.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 1,
          turnId: "t2",
          channelId: "c2",
          timestamp: "2024-01-01T00:01:00Z",
        }),
        makeEvent({
          seq: 2,
          turnId: "t3",
          channelId: "c3",
          timestamp: "2024-01-01T00:01:01Z",
        }),
        makeEvent({
          seq: 3,
          kind: "turn_completed",
          turnId: "t2",
          channelId: "c2",
          timestamp: "2024-01-01T00:01:02Z",
        }),
      ]);
      const channels = getActiveChannelsForAgent(AGENT);
      // t1 still active (not ended), t2 ended, t3 still active.
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

  describe("replay idempotency", () => {
    it("replaying the same buffer produces no additional state change or notifications", () => {
      const buffer = [
        makeEvent({
          seq: 1,
          turnId: "t1",
          channelId: "c1",
          timestamp: "2024-01-01T00:00:00Z",
        }),
        makeEvent({
          seq: 2,
          turnId: "t2",
          channelId: "c2",
          timestamp: "2024-01-01T00:00:01Z",
        }),
      ];

      // Initial pass.
      syncAgentTurnsFromEvents(AGENT, buffer);
      const afterFirst = getActiveChannelsForAgent(AGENT);
      assert.equal(afterFirst.size, 2);

      // Subscribe, then replay the identical buffer.
      let notified = 0;
      const unsub = subscribeActiveAgentTurns(() => {
        notified++;
      });
      syncAgentTurnsFromEvents(AGENT, buffer);
      unsub();

      assert.equal(notified, 0, "replay must not notify listeners");
      const afterReplay = getActiveChannelsForAgent(AGENT);
      assert.equal(
        afterReplay,
        afterFirst,
        "replay must not change turn state (stable reference)",
      );
    });

    it("post-restart replay does not reprocess seen events or resurrect evicted turns", () => {
      // Start a turn, then complete it (turn evicted).
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 1,
          turnId: "t1",
          channelId: "c1",
          timestamp: "2024-01-01T00:00:00Z",
        }),
        makeEvent({
          seq: 2,
          kind: "turn_completed",
          turnId: "t1",
          channelId: "c1",
          timestamp: "2024-01-01T00:00:01Z",
        }),
      ]);
      assert.equal(getActiveChannelsForAgent(AGENT).size, 0);

      // Agent restarts. The harness replays its buffer with seq reset to 1,
      // but the original event timestamps (older than the watermark) are
      // unchanged. The start event must NOT resurrect the evicted turn.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 1,
          turnId: "t1",
          channelId: "c1",
          timestamp: "2024-01-01T00:00:00Z",
        }),
        makeEvent({
          seq: 2,
          kind: "turn_completed",
          turnId: "t1",
          channelId: "c1",
          timestamp: "2024-01-01T00:00:01Z",
        }),
      ]);
      assert.equal(
        getActiveChannelsForAgent(AGENT).size,
        0,
        "stale replayed start must not resurrect an evicted turn",
      );
    });
  });

  describe("replayed eviction safety", () => {
    it("replayed stale turn_error with null turnId does not kill the live turn", () => {
      // A turn errors out (harness emits turn_error with a null turnId), then a
      // fresh turn starts in the same channel.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 1,
          turnId: "t1",
          channelId: "c1",
          timestamp: "2024-01-01T00:00:00Z",
        }),
        makeEvent({
          seq: 2,
          kind: "turn_error",
          turnId: null,
          channelId: "c1",
          timestamp: "2024-01-01T00:00:01Z",
        }),
        makeEvent({
          seq: 3,
          turnId: "t2",
          channelId: "c1",
          timestamp: "2024-01-01T00:00:02Z",
        }),
      ]);
      assert.equal(getActiveChannelsForAgent(AGENT).size, 1);

      // The full buffer is replayed on the next observer event. The stale
      // turn_error (below the watermark) must NOT re-run its channel-match
      // fallback and delete the live turn t2.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 1,
          turnId: "t1",
          channelId: "c1",
          timestamp: "2024-01-01T00:00:00Z",
        }),
        makeEvent({
          seq: 2,
          kind: "turn_error",
          turnId: null,
          channelId: "c1",
          timestamp: "2024-01-01T00:00:01Z",
        }),
        makeEvent({
          seq: 3,
          turnId: "t2",
          channelId: "c1",
          timestamp: "2024-01-01T00:00:02Z",
        }),
      ]);
      const channels = getActiveChannelsForAgent(AGENT);
      assert.equal(
        channels.size,
        1,
        "replayed stale turn_error must not delete the live turn",
      );
      assert.ok(channels.has("c1"));
    });

    it("replaying evictions fires no spurious listener notifications", () => {
      const buffer = [
        makeEvent({
          seq: 1,
          turnId: "t1",
          channelId: "c1",
          timestamp: "2024-01-01T00:00:00Z",
        }),
        makeEvent({
          seq: 2,
          kind: "turn_error",
          turnId: null,
          channelId: "c1",
          timestamp: "2024-01-01T00:00:01Z",
        }),
        makeEvent({
          seq: 3,
          kind: "agent_panic",
          turnId: null,
          channelId: "c2",
          timestamp: "2024-01-01T00:00:02Z",
        }),
      ];

      // Initial pass processes the buffer.
      syncAgentTurnsFromEvents(AGENT, buffer);

      // Subscribe, then replay the identical buffer. Every event is below the
      // watermark, so the replay must be a complete no-op.
      let notified = 0;
      const unsub = subscribeActiveAgentTurns(() => {
        notified++;
      });
      syncAgentTurnsFromEvents(AGENT, buffer);
      unsub();

      assert.equal(notified, 0, "replayed evictions must not notify listeners");
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
