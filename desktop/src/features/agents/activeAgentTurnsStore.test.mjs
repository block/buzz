import assert from "node:assert/strict";
import { describe, it, beforeEach, afterEach, mock } from "node:test";

import {
  syncAgentTurnsFromEvents,
  syncActiveAgentTurnsFromObserver,
  getActiveTurnsForAgent,
  getActiveTurnsByChannel,
  resetActiveAgentTurnsStore,
  subscribeActiveAgentTurns,
  saveActiveAgentTurnsForCommunity,
  restoreActiveAgentTurnsForCommunity,
  clearSavedCommunitySnapshot,
} from "./activeAgentTurnsStore.ts";
import {
  injectObserverEventsForE2E,
  getAgentObserverSnapshot,
  resetAgentObserverStore,
} from "./observerRelayStore.ts";
import { formatElapsed } from "./ui/agentSessionUtils.ts";

const AGENT =
  "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
const AGENT_2 =
  "dcba4321dcba4321dcba4321dcba4321dcba4321dcba4321dcba4321dcba4321";

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

  describe("channel aggregation", () => {
    it("collapses active turns by channel across agents", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 1,
          turnId: "agent-1-early",
          channelId: "shared",
          timestamp: "2024-01-01T00:00:00Z",
        }),
        makeEvent({
          seq: 2,
          turnId: "agent-1-late",
          channelId: "shared",
          timestamp: "2024-01-01T00:01:00Z",
        }),
      ]);
      syncAgentTurnsFromEvents(AGENT_2, [
        makeEvent({
          seq: 1,
          turnId: "agent-2",
          channelId: "shared",
          timestamp: "2024-01-01T00:02:00Z",
        }),
      ]);

      const summaries = getActiveTurnsByChannel();
      assert.deepEqual(
        summaries.map(({ channelId, agentCount, agentPubkeys }) => ({
          channelId,
          agentCount,
          agentPubkeys,
        })),
        [
          {
            channelId: "shared",
            agentCount: 2,
            agentPubkeys: [AGENT, AGENT_2],
          },
        ],
      );
      assert.equal(
        summaries[0].anchorAt,
        getActiveTurnsForAgent(AGENT)[0].anchorAt,
      );
    });

    it("removes a channel summary when the last active turn ends", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
        makeEvent({
          seq: 2,
          kind: "turn_completed",
          turnId: "t1",
          channelId: "c1",
        }),
      ]);

      assert.deepEqual(getActiveTurnsByChannel(), []);
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

    it("agent_panic with an explicit turnId removes only that turn", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
        makeEvent({ seq: 2, turnId: "t2", channelId: "c1" }),
        makeEvent({
          seq: 3,
          kind: "agent_panic",
          turnId: "t2",
          channelId: "c1",
        }),
      ]);

      assert.equal(
        getActiveTurnsForAgent(AGENT).length,
        1,
        "the explicit panic must preserve the other live turn",
      );

      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 4,
          kind: "turn_completed",
          turnId: "t1",
          channelId: "c1",
        }),
      ]);
      assert.equal(getActiveTurnsForAgent(AGENT).length, 0);
    });

    it("turn_error with an explicit turnId removes only that turn", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
        makeEvent({ seq: 2, turnId: "t2", channelId: "c1" }),
        makeEvent({
          seq: 3,
          kind: "turn_error",
          turnId: "t2",
          channelId: "c1",
        }),
      ]);

      assert.equal(
        getActiveTurnsForAgent(AGENT).length,
        1,
        "the explicit error must preserve the other live turn",
      );

      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 4,
          kind: "turn_completed",
          turnId: "t1",
          channelId: "c1",
        }),
      ]);
      assert.equal(getActiveTurnsForAgent(AGENT).length, 0);
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

    it("anchors a turn to its skew-corrected start, not the local insert clock", () => {
      // The badge anchor must reflect the agent's true start translated into
      // desktop time (startedAt + clock offset), so a turn whose event arrives
      // with a stale timestamp does NOT reset to ~Date.now(). With a single
      // event the offset is exactly Date.now() - startedAt, so the anchor lands
      // on Date.now() here — the regression coverage for skew lives below.
      const before = Date.now();
      syncAgentTurnsFromEvents(AGENT, [
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
        summary.anchorAt >= before && summary.anchorAt <= after,
        "anchorAt must be the skew-corrected start, here equal to the local clock",
      );
    });

    it("gives two turns with different startedAt different anchors (no lockstep)", () => {
      // The lockstep bug: turns processed in the same JS tick were all anchored
      // to one shared Date.now(), so their elapsed counters ticked in unison.
      // Anchoring to startedAt + offset makes distinct agent-host starts produce
      // distinct anchors. A single sampleClockOffset minimum is shared, so the
      // anchor difference equals the startedAt difference.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 1,
          turnId: "t1",
          channelId: "c-early",
          timestamp: "2024-01-01T00:00:00Z",
        }),
        makeEvent({
          seq: 2,
          turnId: "t2",
          channelId: "c-late",
          timestamp: "2024-01-01T00:05:00Z",
        }),
      ]);
      const byChannel = new Map(
        getActiveTurnsForAgent(AGENT).map((s) => [s.channelId, s.anchorAt]),
      );
      assert.notEqual(
        byChannel.get("c-early"),
        byChannel.get("c-late"),
        "distinct startedAt must yield distinct anchors",
      );
      assert.equal(
        byChannel.get("c-late") - byChannel.get("c-early"),
        5 * 60_000,
        "anchor spacing must equal the agent-host start spacing",
      );
    });

    it("collapses two turns in one channel to the earliest anchor", () => {
      // Same agent-host start timestamp, distinct turns (seq bumped so the
      // second passes the watermark). Identical timestamps mean the offset does
      // not move, so the surfaced anchor is stable and the earliest wins.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 1,
          turnId: "t1",
          channelId: "c1",
          timestamp: "2024-01-01T00:00:00Z",
        }),
      ]);
      const firstAnchor = getActiveTurnsForAgent(AGENT)[0].anchorAt;
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 2,
          turnId: "t2",
          channelId: "c1",
          timestamp: "2024-01-01T00:00:00Z",
        }),
      ]);
      const summaries = getActiveTurnsForAgent(AGENT);
      assert.equal(summaries.length, 1, "same channel collapses to one entry");
      assert.equal(
        summaries[0].anchorAt,
        firstAnchor,
        "earliest start's anchor must be surfaced",
      );
    });

    it("advances to the surviving turn's anchor after the earliest ends", () => {
      // Two turns in one channel; the array must be rebuilt from the LIVE map
      // on every mutation, so ending the earliest-started turn must surface the
      // survivor's (later) anchor — not a stale cached minimum.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 1,
          turnId: "t-early",
          channelId: "c1",
          timestamp: "2024-01-01T00:00:00Z",
        }),
        makeEvent({
          seq: 2,
          turnId: "t-later",
          channelId: "c1",
          timestamp: "2024-01-01T00:02:00Z",
        }),
      ]);
      const tEarly = getActiveTurnsForAgent(AGENT)[0].anchorAt;

      // End the earliest turn by its turnId. Reuse t-later's timestamp (seq
      // bumped to pass the watermark) so the offset does not tighten and the
      // surviving anchor's advance is exactly the 2-minute start gap.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 3,
          kind: "turn_completed",
          turnId: "t-early",
          channelId: "c1",
          timestamp: "2024-01-01T00:02:00Z",
        }),
      ]);
      const [survivor] = getActiveTurnsForAgent(AGENT);
      assert.equal(survivor.channelId, "c1");
      assert.equal(
        survivor.anchorAt - tEarly,
        2 * 60_000,
        "surfaced anchor must advance to the surviving turn after eviction",
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
    // Mirrors the store's private timing constants. Keep these consumer-level
    // tests deterministic without exporting implementation details.
    const PRUNE_INTERVAL_MS = 5_000;
    const PRUNE_PAUSE_MAX_MS = 3 * 60_000;

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

    it("prunes a stale turn at the bound when its tracked sibling stays fresh", () => {
      // A same-agent tracked sibling keeps the agent's max lastActivityAt
      // fresh, so the stale turn is genuinely dead and must still prune at 25s.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 1,
          turnId: "dead",
          channelId: "c1",
          timestamp: at(0),
        }),
        makeEvent({
          seq: 2,
          turnId: "live",
          channelId: "c2",
          timestamp: at(0),
        }),
      ]);
      assert.equal(getActiveTurnsForAgent(AGENT).length, 2);

      // Keep the live turn fresh across the dead turn's bound: ping every 10s.
      for (let t = 10_000; t <= 30_000; t += 10_000) {
        mock.timers.tick(10_000);
        syncAgentTurnsFromEvents(AGENT, [
          makeEvent({
            seq: 2 + t / 10_000,
            kind: "turn_liveness",
            turnId: "live",
            channelId: "c2",
            timestamp: at(t),
          }),
        ]);
      }

      const channels = channelIdsOf(getActiveTurnsForAgent(AGENT));
      assert.ok(!channels.has("c1"), "the dead turn must prune at the bound");
      assert.ok(channels.has("c2"), "the live sibling must survive");
    });

    it("pauses an agent's stale turns while another agent stays fresh", () => {
      // Agent B keeps reporting tracked-turn activity, but that must not cause
      // agent A's fully stale stream to prune at 25s.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 1,
          turnId: "stale",
          channelId: "a",
          timestamp: at(0),
        }),
      ]);
      syncAgentTurnsFromEvents(AGENT_2, [
        makeEvent({ seq: 1, turnId: "live", channelId: "b", timestamp: at(0) }),
      ]);

      for (let t = 10_000; t <= 30_000; t += 10_000) {
        mock.timers.tick(10_000);
        syncAgentTurnsFromEvents(AGENT_2, [
          makeEvent({
            seq: t / 10_000 + 1,
            kind: "turn_liveness",
            turnId: "live",
            channelId: "b",
            timestamp: at(t),
          }),
        ]);
      }

      assert.ok(
        channelIdsOf(getActiveTurnsForAgent(AGENT)).has("a"),
        "a fully stale agent must stay visible while another agent remains fresh",
      );

      // Keep B's tracked turn fresh until A passes the bounded 3-minute pause.
      for (let t = 40_000; t <= PRUNE_PAUSE_MAX_MS; t += 10_000) {
        mock.timers.tick(10_000);
        syncAgentTurnsFromEvents(AGENT_2, [
          makeEvent({
            seq: t / 10_000 + 1,
            kind: "turn_liveness",
            turnId: "live",
            channelId: "b",
            timestamp: at(t),
          }),
        ]);
      }
      mock.timers.tick(PRUNE_INTERVAL_MS);

      assert.ok(
        !channelIdsOf(getActiveTurnsForAgent(AGENT)).has("a"),
        "the stale agent must prune after the 3-minute pause cap",
      );
      assert.ok(
        channelIdsOf(getActiveTurnsForAgent(AGENT_2)).has("b"),
        "the fresh agent must stay active",
      );
    });

    it("prunes a lone silent turn after the 3-minute pause cap", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1", timestamp: at(0) }),
      ]);

      mock.timers.tick(PRUNE_PAUSE_MAX_MS);

      assert.equal(
        getActiveTurnsForAgent(AGENT).length,
        0,
        "the pause backstop must clear a dead lone agent at the 3-minute cap",
      );
    });

    it("treats a turn_liveness with a null turnId as a no-op", () => {
      // A null-turnId liveness must refresh NOTHING. With a live sibling
      // keeping the max fresh, the dead turn still prunes at the bound — so if
      // the null ping wrongly refreshed the dead turn it would survive here.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 1,
          turnId: "dead",
          channelId: "c1",
          timestamp: at(0),
        }),
        makeEvent({
          seq: 2,
          turnId: "live",
          channelId: "c2",
          timestamp: at(0),
        }),
      ]);

      // A null-turnId liveness for the dead turn must not refresh it. Keep the
      // live sibling pinging so the pause never engages.
      assert.doesNotThrow(() => {
        for (let t = 10_000; t <= 30_000; t += 10_000) {
          mock.timers.tick(10_000);
          syncAgentTurnsFromEvents(AGENT, [
            makeEvent({
              seq: 100 + t,
              kind: "turn_liveness",
              turnId: null,
              channelId: "c1",
              timestamp: at(t),
            }),
            makeEvent({
              seq: 200 + t,
              kind: "turn_liveness",
              turnId: "live",
              channelId: "c2",
              timestamp: at(t),
            }),
          ]);
        }
      });

      const channels = channelIdsOf(getActiveTurnsForAgent(AGENT));
      assert.ok(
        !channels.has("c1"),
        "a null-turnId liveness must not refresh the dead turn, so it prunes",
      );
      assert.ok(channels.has("c2"), "the live sibling must survive");
    });
  });

  describe("skew-corrected elapsed (real-time arrival)", () => {
    // The clock offset estimate (running minimum of Date.now() - event time)
    // is only meaningful when events arrive at distinct real times — exactly
    // how the harness streams them. Faking Date lets us advance the desktop
    // clock between events so an earlier event calibrates the offset before the
    // measured turn starts. The fixed epoch is the desktop clock floor.
    const EPOCH = Date.parse("2024-06-01T00:00:00Z");

    beforeEach(() => {
      mock.timers.enable({ apis: ["Date"], now: EPOCH });
    });

    afterEach(() => {
      mock.timers.reset();
    });

    /** Agent-host clock = desktop clock + skew, as an ISO timestamp. */
    const agentTs = (desktopMs, skew) =>
      new Date(desktopMs + skew).toISOString();

    it("shows a large elapsed for a turn that started well in the past", () => {
      // Clocks synced (skew 0). An early event at the true present calibrates
      // offset ≈ 0. Five true minutes pass. Then the desktop first observes a
      // turn whose start timestamp is that 5-minutes-ago instant — the badge
      // must read ~5 minutes, not reset to 0s on first sight.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 1,
          kind: "turn_liveness",
          turnId: "warm",
          channelId: "c0",
          timestamp: agentTs(EPOCH - 1_000, 0),
        }),
      ]);
      mock.timers.tick(5 * 60_000); // 5 true minutes elapse
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 2,
          turnId: "t1",
          channelId: "c1",
          timestamp: agentTs(EPOCH, 0), // started 5 minutes ago
        }),
      ]);
      const summary = getActiveTurnsForAgent(AGENT).find(
        (s) => s.channelId === "c1",
      );
      assert.equal(
        Date.now() - summary.anchorAt,
        5 * 60_000 - 1_000,
        "a 5-minute-old turn must show ~5 minutes elapsed, not 0s",
      );
    });

    it("corrects for agent-host clock skew so elapsed tracks true duration", () => {
      // Agent host is 1 hour AHEAD of the desktop. A liveness event received at
      // the true present (desktop EPOCH) carries a timestamp an hour in the
      // future, calibrating offset ≈ -1h. The turn then starts 30s later in
      // true time; its future-stamped start, corrected by the offset, anchors
      // to the true start — without correction elapsed would be deeply negative.
      const SKEW = 60 * 60_000;
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 1,
          kind: "turn_liveness",
          turnId: "warm",
          channelId: "c0",
          timestamp: agentTs(EPOCH, SKEW),
        }),
      ]);
      mock.timers.tick(30_000); // 30s of true time passes
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 2,
          turnId: "t1",
          channelId: "c1",
          timestamp: agentTs(EPOCH + 30_000, SKEW),
        }),
      ]);
      const summary = getActiveTurnsForAgent(AGENT).find(
        (s) => s.channelId === "c1",
      );
      assert.equal(
        Date.now() - summary.anchorAt,
        0,
        "a just-started turn under heavy skew must read ~0s, not a negative/huge value",
      );

      // Let the turn run 45s; elapsed must track that true duration exactly.
      mock.timers.tick(45_000);
      const stillRunning = getActiveTurnsForAgent(AGENT).find(
        (s) => s.channelId === "c1",
      );
      assert.equal(
        Date.now() - stillRunning.anchorAt,
        45_000,
        "skew-corrected elapsed must track true duration as the clock advances",
      );
    });

    it("retroactively corrects a live turn's anchor when the offset tightens", () => {
      // The design's load-bearing invariant: anchors are derived at READ time,
      // so a later, tighter offset must shift an ALREADY-LIVE turn earlier.
      // The turn first goes live under a loose offset (its start arrives with a
      // +5s processing delay → offset +5000), then a delay-free liveness sample
      // tightens the running minimum to 0. The live turn's surfaced anchor must
      // move earlier by exactly that 5000ms delta. A regression that froze
      // anchorAt at startTurn would leave the anchor at its loose value and
      // fail this assertion.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 1,
          turnId: "t1",
          channelId: "c1",
          timestamp: agentTs(EPOCH - 5_000, 0), // observed 5s after its start
        }),
      ]);
      const looseAnchor = getActiveTurnsForAgent(AGENT).find(
        (s) => s.channelId === "c1",
      ).anchorAt;

      mock.timers.tick(1_000); // 1s of true time so the liveness arrives later
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 2,
          kind: "turn_liveness",
          turnId: "t1",
          channelId: "c1",
          timestamp: agentTs(EPOCH + 1_000, 0), // delay-free → offset tightens to 0
        }),
      ]);
      const tightAnchor = getActiveTurnsForAgent(AGENT).find(
        (s) => s.channelId === "c1",
      ).anchorAt;

      assert.equal(
        tightAnchor - looseAnchor,
        -5_000,
        "a tighter offset must shift the live turn's read-time anchor earlier by the tightening delta",
      );
    });
  });

  describe("resurrection after a prune (A) gated by completion (C)", () => {
    const EPOCH = Date.parse("2024-01-01T00:00:00Z");
    const at = (ms) => new Date(EPOCH + ms).toISOString();
    const PRUNE_INTERVAL_MS = 5_000;

    let unsubscribe;

    beforeEach(() => {
      mock.timers.enable({ apis: ["setInterval", "Date"], now: EPOCH });
      unsubscribe = subscribeActiveAgentTurns(() => {});
    });

    afterEach(() => {
      unsubscribe();
      mock.timers.reset();
    });

    it("resurrects a pruned turn at startedAt from the first recovered ACP frame", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1", timestamp: at(0) }),
      ]);

      // A fresh tracked sibling permits pruning stale t1 at the normal bound.
      mock.timers.tick(30_000);
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 2,
          turnId: "t2",
          channelId: "c2",
          timestamp: at(30_000),
        }),
      ]);
      mock.timers.tick(PRUNE_INTERVAL_MS);
      assert.ok(
        !channelIdsOf(getActiveTurnsForAgent(AGENT)).has("c1"),
        "the stale turn must prune before its activity recovers",
      );

      mock.timers.tick(10_000);
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 3,
          kind: "acp_read",
          turnId: "t1",
          channelId: "c1",
          timestamp: at(45_000),
          startedAt: at(0),
        }),
      ]);

      const resurrected = getActiveTurnsForAgent(AGENT).find(
        (turn) => turn.channelId === "c1",
      );
      assert.equal(
        Date.now() - resurrected.anchorAt,
        45_000,
        "the first recovered ACP frame must preserve elapsed time from the original turn start",
      );
    });

    it("preserves a valid Unix-epoch startedAt envelope timestamp", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 1,
          kind: "turn_liveness",
          turnId: "t1",
          channelId: "c1",
          timestamp: at(0),
          startedAt: "1970-01-01T00:00:00.000Z",
        }),
      ]);

      const [resurrected] = getActiveTurnsForAgent(AGENT);
      assert.equal(
        resurrected.anchorAt,
        0,
        "a valid zero timestamp must not be replaced with the recovery time",
      );
    });

    it("falls back to the frame timestamp when startedAt is absent or invalid", () => {
      for (const invalidStartedAt of [
        undefined,
        null,
        "not-a-timestamp",
        "future",
      ]) {
        resetActiveAgentTurnsStore();
        const startedAtMs = Date.now();
        const iso = (offset) => new Date(startedAtMs + offset).toISOString();
        const startedAt =
          invalidStartedAt === "future" ? iso(46_000) : invalidStartedAt;

        syncAgentTurnsFromEvents(AGENT, [
          makeEvent({
            seq: 1,
            turnId: "t1",
            channelId: "c1",
            timestamp: iso(0),
          }),
        ]);
        mock.timers.tick(30_000);
        syncAgentTurnsFromEvents(AGENT, [
          makeEvent({
            seq: 2,
            turnId: "t2",
            channelId: "c2",
            timestamp: iso(30_000),
          }),
        ]);
        mock.timers.tick(PRUNE_INTERVAL_MS);
        mock.timers.tick(10_000);
        syncAgentTurnsFromEvents(AGENT, [
          makeEvent({
            seq: 3,
            kind: "turn_liveness",
            turnId: "t1",
            channelId: "c1",
            timestamp: iso(45_000),
            startedAt,
          }),
        ]);

        const resurrected = getActiveTurnsForAgent(AGENT).find(
          (turn) => turn.channelId === "c1",
        );
        assert.equal(
          Date.now() - resurrected.anchorAt,
          0,
          "old or malformed frames must retain the existing recovery anchor",
        );
      }
    });

    it("does NOT resurrect a turn whose liveness is older than its completion", () => {
      // Bound-proving (stale side): a turn completes, then a liveness frame
      // arrives carrying a timestamp BEFORE the completion. It must not revive.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1", timestamp: at(0) }),
        makeEvent({
          seq: 2,
          kind: "turn_completed",
          turnId: "t1",
          channelId: "c1",
          timestamp: at(10_000),
        }),
      ]);
      assert.equal(getActiveTurnsForAgent(AGENT).length, 0);

      // A liveness stamped at 5s (before the 10s completion) but delivered with
      // a later seq so it clears the watermark on seq. It is stale relative to
      // the completion and must NOT resurrect.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 3,
          kind: "turn_liveness",
          turnId: "t1",
          channelId: "c1",
          timestamp: at(5_000),
        }),
      ]);
      assert.equal(
        getActiveTurnsForAgent(AGENT).length,
        0,
        "a liveness older than the recorded completion must not resurrect the turn",
      );
    });

    it("DOES resurrect a turn whose liveness is strictly newer than its completion", () => {
      // Bound-proving (live side): the same completed turn, but a liveness frame
      // strictly NEWER than the completion (a genuine restart of the same id)
      // must revive — the completion only blocks stale frames, not new work.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1", timestamp: at(0) }),
        makeEvent({
          seq: 2,
          kind: "turn_completed",
          turnId: "t1",
          channelId: "c1",
          timestamp: at(10_000),
        }),
      ]);
      assert.equal(getActiveTurnsForAgent(AGENT).length, 0);

      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 3,
          kind: "turn_liveness",
          turnId: "t1",
          channelId: "c1",
          timestamp: at(20_000),
        }),
      ]);
      assert.ok(
        channelIdsOf(getActiveTurnsForAgent(AGENT)).has("c1"),
        "a liveness strictly newer than the completion must resurrect the turn",
      );
    });

    it("does NOT resurrect from a liveness frame with no channelId", () => {
      // A pruned turn cannot be rebuilt without a channelId to anchor the badge.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1", timestamp: at(0) }),
        makeEvent({
          seq: 2,
          turnId: "t2",
          channelId: "c2",
          timestamp: at(0),
        }),
      ]);
      // Drop t1 by ending it, then send a channelId-less liveness for it.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 3,
          kind: "turn_completed",
          turnId: "t1",
          channelId: "c1",
          timestamp: at(5_000),
        }),
        makeEvent({
          seq: 4,
          kind: "turn_liveness",
          turnId: "t1",
          channelId: null,
          timestamp: at(10_000),
        }),
      ]);
      assert.ok(
        !channelIdsOf(getActiveTurnsForAgent(AGENT)).has("c1"),
        "a channelId-less liveness cannot resurrect a badge",
      );
    });

    it("clears completion tombstones on reset so a later turn can run", () => {
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({ seq: 1, turnId: "t1", channelId: "c1", timestamp: at(0) }),
        makeEvent({
          seq: 2,
          kind: "turn_completed",
          turnId: "t1",
          channelId: "c1",
          timestamp: at(10_000),
        }),
      ]);

      resetActiveAgentTurnsStore();

      // After reset, an OLD-stamped liveness for the same id must resurrect,
      // proving the tombstone (which would otherwise block it) was cleared.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: 1,
          kind: "turn_liveness",
          turnId: "t1",
          channelId: "c1",
          timestamp: at(1_000),
        }),
      ]);
      assert.ok(
        channelIdsOf(getActiveTurnsForAgent(AGENT)).has("c1"),
        "reset must clear terminal tombstones so they do not leak across reset",
      );
    });

    it("evicts the oldest tombstone once past the cap so the map stays bounded", () => {
      // The tombstone map is capped at MAX_TERMINAL_TOMBSTONES (16). Complete
      // 18 distinct turns so eviction fires twice, dropping the two oldest by
      // insertion order (t0, t1). Probe via the ONE behavior a tombstone gates
      // that a strictly-newer frame cannot mask: an EQUAL-timestamp liveness
      // (frameAt == terminalAt). All completions share timestamp T with rising
      // seq, so the probe clears the per-agent watermark on the seq tiebreak
      // (compareObserverEvents is timestamp-primary, seq-secondary) yet stays
      // equal to the recorded terminal — reaching resurrectTurn's tombstone
      // check rather than being shadowed by the watermark.
      const CAP = 16;
      const TOTAL = CAP + 2;
      const T = at(0);
      const completions = [];
      for (let i = 0; i < TOTAL; i++) {
        completions.push(
          makeEvent({
            seq: i + 1,
            kind: "turn_completed",
            turnId: `t${i}`,
            channelId: `c${i}`,
            timestamp: T,
          }),
        );
      }
      syncAgentTurnsFromEvents(AGENT, completions);

      // A surviving tombstone (t2, third-completed) still blocks an
      // equal-timestamp liveness — proves the tombstone is present and doing
      // the work the watermark cannot.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: TOTAL + 1,
          kind: "turn_liveness",
          turnId: "t2",
          channelId: "c2",
          timestamp: T,
        }),
      ]);
      assert.ok(
        !channelIdsOf(getActiveTurnsForAgent(AGENT)).has("c2"),
        "a surviving tombstone must still block an equal-timestamp liveness",
      );

      // The oldest tombstone (t0) was evicted, so the same equal-timestamp
      // liveness now resurrects — proving the cap fired AND evicted the
      // oldest-by-insertion entry, not an arbitrary one.
      syncAgentTurnsFromEvents(AGENT, [
        makeEvent({
          seq: TOTAL + 2,
          kind: "turn_liveness",
          turnId: "t0",
          channelId: "c0",
          timestamp: T,
        }),
      ]);
      assert.ok(
        channelIdsOf(getActiveTurnsForAgent(AGENT)).has("c0"),
        "the oldest tombstone must be evicted once past the cap",
      );
    });
  });
});

/**
 * Regression: raw observer ingestion and derived active-turn liveness must
 * stay in sync. The channel activity path previously mounted only the observer
 * bridge, so a raw `turn_completed` could be visible in the activity feed
 * while the derived liveness indicator kept spinning. This drives events
 * through the real observer store (appendAgentEvent path) and the same sync
 * the bridge hook runs, asserting derived liveness clears with the raw feed.
 */
describe("observer → active-turns bridge sync", () => {
  const bridgeAgents = [{ pubkey: AGENT, status: "deployed" }];

  beforeEach(() => {
    resetActiveAgentTurnsStore();
    resetAgentObserverStore();
  });

  afterEach(() => {
    resetAgentObserverStore();
  });

  it("clears derived liveness when raw turn_completed arrives", () => {
    injectObserverEventsForE2E(AGENT, [
      makeEvent({ seq: 1, kind: "turn_started" }),
    ]);
    syncActiveAgentTurnsFromObserver(bridgeAgents);
    assert.ok(
      channelIdsOf(getActiveTurnsForAgent(AGENT)).has("chan-1"),
      "turn_started must surface an active turn",
    );

    injectObserverEventsForE2E(AGENT, [
      makeEvent({
        seq: 2,
        kind: "turn_completed",
        timestamp: "2024-01-01T00:00:05Z",
      }),
    ]);
    syncActiveAgentTurnsFromObserver(bridgeAgents);

    const rawEvents = getAgentObserverSnapshot(AGENT, true).events;
    assert.equal(
      rawEvents.at(-1)?.kind,
      "turn_completed",
      "raw feed must contain the completion event",
    );
    assert.equal(
      getActiveTurnsForAgent(AGENT).length,
      0,
      "derived liveness must clear when the raw feed shows turn_completed",
    );
  });

  it("skips agents that are neither running nor deployed", () => {
    injectObserverEventsForE2E(AGENT, [
      makeEvent({ seq: 1, kind: "turn_started" }),
    ]);
    syncActiveAgentTurnsFromObserver([{ pubkey: AGENT, status: "stopped" }]);
    assert.equal(
      getActiveTurnsForAgent(AGENT).length,
      0,
      "inactive agents must not populate the active-turns store",
    );
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

describe("community-switch save / restore", () => {
  beforeEach(() => {
    resetActiveAgentTurnsStore();
  });

  it("restores original startedAt timestamps across a round-trip", () => {
    // Simulate a turn active in community A with a known start timestamp.
    syncAgentTurnsFromEvents(AGENT, [
      makeEvent({
        seq: 1,
        turnId: "t1",
        channelId: "c1",
        timestamp: "2024-01-01T00:00:00Z",
      }),
    ]);
    const anchorBefore = getActiveTurnsForAgent(AGENT)[0].anchorAt;

    // Switch away (save A, reset, apply B).
    saveActiveAgentTurnsForCommunity("ws-a");
    resetActiveAgentTurnsStore();
    assert.equal(
      getActiveTurnsForAgent(AGENT).length,
      0,
      "store must be empty after reset",
    );

    // Switch back (restore A).
    restoreActiveAgentTurnsForCommunity("ws-a");
    const turns = getActiveTurnsForAgent(AGENT);
    assert.equal(turns.length, 1, "restored turn must reappear");
    assert.equal(turns[0].channelId, "c1");
    assert.equal(
      turns[0].anchorAt,
      anchorBefore,
      "anchorAt must equal the pre-switch value — startedAt and offset are preserved",
    );
  });

  it("no-op restore when no snapshot exists for the community", () => {
    // Populate the store so we can verify nothing changes.
    syncAgentTurnsFromEvents(AGENT, [
      makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
    ]);
    let notified = 0;
    const unsub = subscribeActiveAgentTurns(() => {
      notified++;
    });
    restoreActiveAgentTurnsForCommunity("ws-unknown");
    unsub();
    assert.equal(notified, 0, "no-op restore must not notify listeners");
    // Store should be unchanged — still has the turn we set up above.
    assert.equal(getActiveTurnsForAgent(AGENT).length, 1);
  });

  it("empty store discards a prior snapshot rather than saving an empty one", () => {
    // First round-trip: save something.
    syncAgentTurnsFromEvents(AGENT, [
      makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
    ]);
    saveActiveAgentTurnsForCommunity("ws-a");

    // Second pass: the turns ended while we were away, so the store is empty
    // when we try to save again.
    resetActiveAgentTurnsStore();
    saveActiveAgentTurnsForCommunity("ws-a"); // empty store → delete snapshot

    // Restore must be a no-op now.
    restoreActiveAgentTurnsForCommunity("ws-a");
    assert.equal(
      getActiveTurnsForAgent(AGENT).length,
      0,
      "restoring after an empty save must yield no turns",
    );
  });

  it("snapshot is consumed on restore — a second restore is a no-op", () => {
    syncAgentTurnsFromEvents(AGENT, [
      makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
    ]);
    saveActiveAgentTurnsForCommunity("ws-a");
    resetActiveAgentTurnsStore();

    restoreActiveAgentTurnsForCommunity("ws-a");
    assert.equal(getActiveTurnsForAgent(AGENT).length, 1, "first restore ok");

    // Second restore — snapshot was consumed.
    resetActiveAgentTurnsStore();
    restoreActiveAgentTurnsForCommunity("ws-a");
    assert.equal(
      getActiveTurnsForAgent(AGENT).length,
      0,
      "second restore must be no-op — snapshot is consumed on first use",
    );
  });

  it("watermark is preserved — events already processed before save are not reprocessed", () => {
    syncAgentTurnsFromEvents(AGENT, [
      makeEvent({
        seq: 5,
        turnId: "t1",
        channelId: "c1",
        timestamp: "2024-01-01T00:00:00Z",
      }),
    ]);
    saveActiveAgentTurnsForCommunity("ws-a");
    resetActiveAgentTurnsStore();
    restoreActiveAgentTurnsForCommunity("ws-a");

    // Replaying an event at or below the watermark must be a no-op.
    let notified = 0;
    const unsub = subscribeActiveAgentTurns(() => {
      notified++;
    });
    syncAgentTurnsFromEvents(AGENT, [
      makeEvent({
        seq: 3,
        turnId: "t2",
        channelId: "c2",
        timestamp: "2024-01-01T00:00:00Z",
      }),
    ]);
    unsub();

    assert.equal(notified, 0, "stale event after restore must not notify");
    const channels = new Set(
      getActiveTurnsForAgent(AGENT).map((s) => s.channelId),
    );
    assert.ok(!channels.has("c2"), "stale event must not add a turn");
  });

  it("terminal tombstones are preserved — a stale liveness cannot resurrect a completed turn", () => {
    // Complete a turn before saving.
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
        timestamp: "2024-01-01T00:00:10Z",
      }),
    ]);
    saveActiveAgentTurnsForCommunity("ws-a");
    resetActiveAgentTurnsStore();
    restoreActiveAgentTurnsForCommunity("ws-a");

    // A liveness stamped before the completion must not resurrect t1.
    syncAgentTurnsFromEvents(AGENT, [
      makeEvent({
        seq: 3,
        kind: "turn_liveness",
        turnId: "t1",
        channelId: "c1",
        timestamp: "2024-01-01T00:00:05Z",
      }),
    ]);
    assert.equal(
      getActiveTurnsForAgent(AGENT).length,
      0,
      "terminal tombstone must survive save/restore — stale liveness must not resurrect",
    );
  });

  it("notifies listeners after restore so UI re-renders with recovered turns", () => {
    syncAgentTurnsFromEvents(AGENT, [
      makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
    ]);
    saveActiveAgentTurnsForCommunity("ws-a");
    resetActiveAgentTurnsStore();

    let notified = 0;
    const unsub = subscribeActiveAgentTurns(() => {
      notified++;
    });
    restoreActiveAgentTurnsForCommunity("ws-a");
    unsub();

    assert.ok(notified > 0, "restore must notify listeners");
  });

  it("multiple communities maintain independent snapshots", () => {
    // Community A: agent in c1.
    syncAgentTurnsFromEvents(AGENT, [
      makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
    ]);
    saveActiveAgentTurnsForCommunity("ws-a");

    // Community B: different agent/channel.
    resetActiveAgentTurnsStore();
    syncAgentTurnsFromEvents(AGENT_2, [
      makeEvent({ seq: 1, turnId: "t2", channelId: "c2" }),
    ]);
    saveActiveAgentTurnsForCommunity("ws-b");

    // Switch to A — only A's turns must appear.
    resetActiveAgentTurnsStore();
    restoreActiveAgentTurnsForCommunity("ws-a");
    const aChannels = new Set(
      getActiveTurnsForAgent(AGENT).map((s) => s.channelId),
    );
    assert.ok(aChannels.has("c1"), "ws-a must restore c1");
    assert.equal(
      getActiveTurnsForAgent(AGENT_2).length,
      0,
      "ws-b turns must not appear in ws-a restore",
    );

    // Switch to B — only B's turns.
    resetActiveAgentTurnsStore();
    restoreActiveAgentTurnsForCommunity("ws-b");
    const bChannels = new Set(
      getActiveTurnsForAgent(AGENT_2).map((s) => s.channelId),
    );
    assert.ok(bChannels.has("c2"), "ws-b must restore c2");
  });

  it("clearSavedCommunitySnapshot discards the snapshot so restore is a no-op", () => {
    syncAgentTurnsFromEvents(AGENT, [
      makeEvent({ seq: 1, turnId: "t1", channelId: "c1" }),
    ]);
    saveActiveAgentTurnsForCommunity("ws-a");

    // Simulate community deletion: GC the snapshot before switching back.
    clearSavedCommunitySnapshot("ws-a");

    resetActiveAgentTurnsStore();
    restoreActiveAgentTurnsForCommunity("ws-a");
    assert.equal(
      getActiveTurnsForAgent(AGENT).length,
      0,
      "restore must be no-op after clearSavedCommunitySnapshot",
    );
  });
});
