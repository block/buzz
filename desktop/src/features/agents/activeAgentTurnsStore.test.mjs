import assert from "node:assert/strict";
import { describe, it, beforeEach, afterEach, mock } from "node:test";

import {
  syncAgentTurnsFromEvents,
  getActiveTurnsForAgent,
  resetActiveAgentTurnsStore,
  subscribeActiveAgentTurns,
} from "./activeAgentTurnsStore.ts";
import { formatElapsed } from "./ui/agentSessionUtils.ts";

const AGENT =
  "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";

/** Channel-id Set view of the summary array — keeps legacy assertions terse. */
function channelIdsOf(turns) {
  return new Set(turns.map((t) => t.channelId));
}

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
      const channels = channelIdsOf(getActiveTurnsForAgent(AGENT));
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
      const channels = channelIdsOf(getActiveTurnsForAgent(AGENT));
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
      const channels = channelIdsOf(getActiveTurnsForAgent(AGENT));
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
      assert.equal(getActiveTurnsForAgent(AGENT).length, 1);

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
      const channels = channelIdsOf(getActiveTurnsForAgent(AGENT));
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
      const channels = channelIdsOf(getActiveTurnsForAgent(AGENT));
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
      const channels = channelIdsOf(getActiveTurnsForAgent(AGENT));
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
      assert.equal(getActiveTurnsForAgent(AGENT).length, 0);
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
      assert.equal(getActiveTurnsForAgent(AGENT).length, 0);
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
      assert.equal(getActiveTurnsForAgent(AGENT).length, 1);
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
      const channels = channelIdsOf(getActiveTurnsForAgent(AGENT));
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
      const afterFirst = getActiveTurnsForAgent(AGENT);
      assert.equal(afterFirst.length, 2);

      // Subscribe, then replay the identical buffer.
      let notified = 0;
      const unsub = subscribeActiveAgentTurns(() => {
        notified++;
      });
      syncAgentTurnsFromEvents(AGENT, buffer);
      unsub();

      assert.equal(notified, 0, "replay must not notify listeners");
      const afterReplay = getActiveTurnsForAgent(AGENT);
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
      assert.equal(getActiveTurnsForAgent(AGENT).length, 0);

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
        getActiveTurnsForAgent(AGENT).length,
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
      assert.equal(getActiveTurnsForAgent(AGENT).length, 1);

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
      const channels = channelIdsOf(getActiveTurnsForAgent(AGENT));
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

  describe("getActiveTurnsForAgent", () => {
    it("returns empty array for null/undefined pubkey", () => {
      assert.equal(getActiveTurnsForAgent(null).length, 0);
      assert.equal(getActiveTurnsForAgent(undefined).length, 0);
    });

    it("returns stable reference when unchanged", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
      ]);
      const ref1 = getActiveTurnsForAgent(AGENT);
      const ref2 = getActiveTurnsForAgent(AGENT);
      assert.equal(ref1, ref2, "should return cached array reference");
    });

    it("preserves a desktop-clock observedAt per channel", () => {
      const before = Date.now();
      syncAgentTurnsFromEvents(AGENT, [
        // startedAt comes from the (stale) event timestamp; observedAt must
        // instead anchor to the local clock at insert time.
        makeEvent({
          seq: 1,
          turnId: "t1",
          channelId: "c1",
          timestamp: "2000-01-01T00:00:00Z",
        }),
      ]);
      const after = Date.now();
      const [summary] = getActiveTurnsForAgent(AGENT);
      assert.equal(summary.channelId, "c1");
      assert.ok(
        summary.observedAt >= before && summary.observedAt <= after,
        "observedAt must be the local clock at insert, not the event timestamp",
      );
    });

    it("collapses two turns in one channel to the earliest observedAt", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
      ]);
      const firstObservedAt = getActiveTurnsForAgent(AGENT)[0].observedAt;

      // Second turn in the same channel — its observedAt is >= the first
      // because the clock is monotonic, so the earliest must still win.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 2, turnId: "t2", channelId: "c1" }),
      ]);
      const summaries = getActiveTurnsForAgent(AGENT);
      assert.equal(summaries.length, 1, "same channel collapses to one entry");
      assert.equal(
        summaries[0].observedAt,
        firstObservedAt,
        "earliest observedAt for the channel must be surfaced",
      );
    });

    it("advances to the surviving turn's observedAt after the earliest ends", () => {
      // Two turns in one channel; the array must be rebuilt from the LIVE map
      // on every mutation, so ending the earliest-observed turn must surface
      // the survivor's observedAt — not a stale cached minimum.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t-early", channelId: "c1" }),
      ]);
      const tEarly = getActiveTurnsForAgent(AGENT)[0].observedAt;

      // Force the second turn's observedAt strictly past the first so the
      // advance is observable even when Date.now() would otherwise collide.
      const spinUntil = Date.now() + 2;
      while (Date.now() < spinUntil) {
        /* busy-wait one clock tick */
      }
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 2, turnId: "t-later", channelId: "c1" }),
      ]);
      assert.equal(
        getActiveTurnsForAgent(AGENT)[0].observedAt,
        tEarly,
        "earliest wins while both turns survive",
      );

      // End the earliest turn by its turnId.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 3,
          kind: "turn_completed",
          turnId: "t-early",
          channelId: "c1",
        }),
      ]);
      const [survivor] = getActiveTurnsForAgent(AGENT);
      assert.equal(survivor.channelId, "c1");
      assert.ok(
        survivor.observedAt > tEarly,
        "surfaced observedAt must advance to the surviving turn after eviction",
      );
    });

    it("sorts summaries by channelId", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c-zebra" }),
        makeEvent({ seq: 2, turnId: "t2", channelId: "c-alpha" }),
      ]);
      const ids = getActiveTurnsForAgent(AGENT).map((s) => s.channelId);
      assert.deepEqual(ids, ["c-alpha", "c-zebra"]);
    });
  });

  describe("turn_liveness prune backstop", () => {
    // The prune sweep runs on an internal setInterval keyed off Date.now();
    // faking both lets us drive the 25s bound deterministically. The fixed
    // epoch is the clock floor — event timestamps below anchor lastActivityAt
    // to it, so elapsed time is exactly what mock.timers.tick advances.
    const EPOCH = Date.parse("2024-01-01T00:00:00Z");
    const at = (ms) => new Date(EPOCH + ms).toISOString();
    // Mirrors the store's REMOVE_AFTER_MS (LIVENESS_INTERVAL_MS * 2.5) and
    // PRUNE_INTERVAL_MS. Not exported — kept in lockstep here so the prune
    // bound stays asserted from the consumer's perspective.
    const REMOVE_AFTER_MS = 25_000;
    const PRUNE_INTERVAL_MS = 5_000;

    let unsubscribe;

    beforeEach(() => {
      mock.timers.enable({ apis: ["setInterval", "Date"], now: EPOCH });
      // Subscribing starts the prune interval under the faked clock.
      unsubscribe = subscribeActiveAgentTurns(() => {});
    });

    afterEach(() => {
      unsubscribe();
      mock.timers.reset();
    });

    it("keeps a turn alive when turn_liveness refreshes before the bound", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1", timestamp: at(0) }),
      ]);

      // Refresh activity at 20s — under the 25s bound — then advance to 40s.
      // Without the refresh the turn would have been pruned by 25s; the
      // liveness ping resets lastActivityAt so it survives.
      mock.timers.tick(20_000);
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 2,
          kind: "turn_liveness",
          turnId: "t1",
          channelId: "c1",
          timestamp: at(20_000),
        }),
      ]);
      mock.timers.tick(20_000);

      const channels = channelIdsOf(getActiveTurnsForAgent(AGENT));
      assert.ok(
        channels.has("c1"),
        "liveness within the bound must defer the prune",
      );
    });

    it("prunes a turn that receives no activity past the bound", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1", timestamp: at(0) }),
      ]);
      assert.equal(getActiveTurnsForAgent(AGENT).length, 1);

      // No liveness pings — the host died without unwinding. Advance past the
      // 25s bound; the next prune sweep evicts the silent turn.
      mock.timers.tick(REMOVE_AFTER_MS + PRUNE_INTERVAL_MS);

      assert.equal(
        getActiveTurnsForAgent(AGENT).length,
        0,
        "a turn with no activity past the bound must be pruned",
      );
    });

    it("treats a turn_liveness with a null turnId as a no-op", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1", timestamp: at(0) }),
      ]);

      // A liveness ping with no turnId must refresh nothing (recordActivity
      // no-ops on null). If it wrongly refreshed, the turn would survive the
      // bound below — so the prune is the observable proof of the no-op, and
      // the missing turnId must not throw.
      mock.timers.tick(20_000);
      assert.doesNotThrow(() => {
        syncAgentTurnsFromEvents(AGENT, [
          makeEvent({
            seq: 2,
            kind: "turn_liveness",
            turnId: null,
            channelId: "c1",
            timestamp: at(20_000),
          }),
        ]);
      });
      mock.timers.tick(REMOVE_AFTER_MS + PRUNE_INTERVAL_MS);

      assert.equal(
        getActiveTurnsForAgent(AGENT).length,
        0,
        "a null-turnId liveness must not refresh activity, so the turn still prunes",
      );
    });
  });
});

describe("formatElapsed", () => {
  it("renders sub-10s as whole seconds", () => {
    assert.equal(formatElapsed(0), "0s");
    assert.equal(formatElapsed(4_900), "4s");
  });

  it("renders sub-minute as whole seconds", () => {
    assert.equal(formatElapsed(59_000), "59s");
  });

  it("rolls into minutes at exactly 60s", () => {
    assert.equal(formatElapsed(60_000), "1m 0s");
  });

  it("renders minutes and seconds", () => {
    assert.equal(formatElapsed(83_000), "1m 23s");
  });

  it("rolls 59m 59s cleanly into 1h 0m 0s at 3600s", () => {
    assert.equal(formatElapsed(3_599_000), "59m 59s");
    assert.equal(formatElapsed(3_600_000), "1h 0m 0s");
  });

  it("clamps negative input to 0s", () => {
    assert.equal(formatElapsed(-5_000), "0s");
  });
});
