/**
 * Retention behavior of the per-agent live observer journal.
 *
 * `appendAgentEvents` derives the transcript incrementally when a batch lands
 * after the retained window, and falls back to a full `buildTranscriptState`
 * replay when the window is evicted. Evicting back to *exactly* the cap made
 * that fallback permanent: an agent parked at the cap evicts one event on every
 * append, so `trimmed` is true forever and every steady-state append replays the
 * whole history through `buildTranscriptState` (issue #5718: 188x headless,
 * live CPU escalating to 119%/core after 5min idle).
 *
 * The fix evicts to a low-water mark below the cap, so the window must refill
 * through ordinary appends before the next eviction — one replay amortized
 * across the refill. Transcript content is identical on the fold and rebuild
 * paths (that is the point of the fallback), so these tests assert the
 * observable that distinguishes them: the retained window's SHAPE after
 * eviction. They also pin the invariant that the derived transcript still equals
 * a full replay of the retained window regardless of which path ran.
 */

import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import {
  getAgentObserverSnapshot,
  getAgentTranscript,
  resetAgentObserverStore,
  subscribeAgentObserverStore,
  syncAgentObserverEvents,
} from "@/features/agents/observerRelayStore.ts";
import { buildTranscript } from "@/features/agents/ui/agentSessionTranscript.ts";

// Mirrors the private constants in observerRelayStore.ts. LOW_WATER is
// Math.floor(MAX * 0.9); the tests assert exact shapes against these values so
// a regression in the eviction math (e.g. reverting to trim-to-cap) fails here.
const MAX_OBSERVER_EVENTS = 3000;
const OBSERVER_EVENTS_LOW_WATER = Math.floor(MAX_OBSERVER_EVENTS * 0.9);

const AGENT_PUBKEY = "a".repeat(64);

/** One live observer event; monotonic timestamp keyed to seq so the store's
 *  timestamp-then-seq sort matches insertion order. */
function makeEvent(seq) {
  return {
    seq,
    timestamp: new Date(1_760_000_000_000 + seq * 1000).toISOString(),
    kind: "turn_started",
    agentIndex: 0,
    channelId: "chan-1",
    sessionId: "sess-1",
    turnId: `turn-${seq}`,
    payload: {},
  };
}

function windowLength() {
  return getAgentObserverSnapshot(AGENT_PUBKEY).events.length;
}

/** Append events seq 1..count one at a time, mirroring the live relay path
 *  where each frame appends and notifies individually. */
function fillSequential(count) {
  for (let seq = 1; seq <= count; seq += 1) {
    syncAgentObserverEvents(AGENT_PUBKEY, [makeEvent(seq)]);
  }
}

describe("live observer journal retention — amortized eviction", () => {
  beforeEach(() => {
    resetAgentObserverStore();
  });

  it("test_window_never_exceeds_cap", () => {
    for (let seq = 1; seq <= MAX_OBSERVER_EVENTS + 750; seq += 1) {
      syncAgentObserverEvents(AGENT_PUBKEY, [makeEvent(seq)]);
      assert.ok(
        windowLength() <= MAX_OBSERVER_EVENTS,
        `window grew to ${windowLength()} at seq ${seq}`,
      );
    }
  });

  it("test_append_at_cap_does_not_trim_prematurely", () => {
    // Filling to exactly the cap must NOT evict — `trimmed` is `length > cap`,
    // and length === cap is not over. A premature trim here would mean the
    // fraction math or the comparison regressed.
    fillSequential(MAX_OBSERVER_EVENTS);
    assert.equal(
      windowLength(),
      MAX_OBSERVER_EVENTS,
      "reaching exactly the cap retains the full window, no eviction",
    );
  });

  it("test_append_crossing_cap_trims_to_exactly_low_water", () => {
    // The append that pushes past the cap must leave the window at exactly the
    // low-water mark — not back at the cap (which would re-arm eviction on the
    // very next append and keep the transcript rebuilding forever).
    fillSequential(MAX_OBSERVER_EVENTS);
    syncAgentObserverEvents(AGENT_PUBKEY, [makeEvent(MAX_OBSERVER_EVENTS + 1)]);

    assert.equal(
      windowLength(),
      OBSERVER_EVENTS_LOW_WATER,
      "crossing the cap trims to exactly the low-water mark, leaving headroom",
    );
    assert.ok(
      windowLength() < MAX_OBSERVER_EVENTS,
      "headroom exists below the cap after eviction",
    );
  });

  it("test_headroom_refills_before_next_eviction", () => {
    // After the first eviction leaves headroom, subsequent appends must GROW
    // the window (no eviction) until it refills to the cap — proving eviction
    // is amortized across the refill, not per-append.
    fillSequential(MAX_OBSERVER_EVENTS + 1);
    assert.equal(windowLength(), OBSERVER_EVENTS_LOW_WATER);

    const headroom = MAX_OBSERVER_EVENTS - OBSERVER_EVENTS_LOW_WATER;
    for (let i = 1; i <= headroom; i += 1) {
      syncAgentObserverEvents(AGENT_PUBKEY, [
        makeEvent(MAX_OBSERVER_EVENTS + 1 + i),
      ]);
      assert.equal(
        windowLength(),
        OBSERVER_EVENTS_LOW_WATER + i,
        `append ${i} into the headroom must grow the window, not evict`,
      );
    }
    // The window is now back at the cap; the next append evicts again.
    syncAgentObserverEvents(AGENT_PUBKEY, [
      makeEvent(MAX_OBSERVER_EVENTS + 2 + headroom),
    ]);
    assert.equal(
      windowLength(),
      OBSERVER_EVENTS_LOW_WATER,
      "the window only evicts again after the headroom is refilled",
    );
  });

  it("test_eviction_keeps_newest_events_drops_oldest", () => {
    const total = MAX_OBSERVER_EVENTS + 400;
    fillSequential(total);
    const events = getAgentObserverSnapshot(AGENT_PUBKEY).events;
    assert.equal(events.at(-1).seq, total, "newest event is retained");
    assert.equal(
      events.at(0).seq,
      total - events.length + 1,
      "retention is the newest-N contiguous tail",
    );
  });

  it("test_derived_transcript_equals_full_replay_after_eviction", () => {
    // The rebuild fallback and the incremental fold must agree: after crossing
    // the cap (rebuild path) the stored transcript equals a fresh replay of the
    // retained window.
    fillSequential(MAX_OBSERVER_EVENTS + 600);
    const retained = getAgentObserverSnapshot(AGENT_PUBKEY).events;
    assert.deepEqual(
      getAgentTranscript(AGENT_PUBKEY),
      buildTranscript(retained),
      "the derived transcript matches a full replay of the retained window",
    );
  });

  it("test_single_batch_larger_than_cap_trims_to_low_water", () => {
    const batch = [];
    for (let seq = 1; seq <= MAX_OBSERVER_EVENTS + 900; seq += 1) {
      batch.push(makeEvent(seq));
    }
    syncAgentObserverEvents(AGENT_PUBKEY, batch);
    assert.equal(
      windowLength(),
      OBSERVER_EVENTS_LOW_WATER,
      "a single over-cap batch also trims to the low-water mark",
    );
    assert.equal(
      getAgentObserverSnapshot(AGENT_PUBKEY).events.at(-1).seq,
      MAX_OBSERVER_EVENTS + 900,
      "the newest event of an over-cap batch is retained",
    );
  });
});

describe("live observer journal retention — eviction floor (reconnect replay)", () => {
  // The dedup set is built only from the retained array, so once eviction
  // discards the oldest frames the journal no longer remembers them. Relay
  // reconnect replays old frames as a normal behavior; without a floor a
  // replayed pre-eviction frame is re-admitted into the headroom and a later
  // refill to the cap trims away legitimate retained events with no new
  // activity. These pin the floor that rejects already-evicted history.
  beforeEach(() => {
    resetAgentObserverStore();
  });

  it("test_replayed_pre_floor_frames_do_not_change_retained_window", () => {
    // Overflow to the low-water mark, then replay the discarded oldest frames.
    // They are at or below the eviction floor, so none is re-admitted: the
    // retained window is byte-identical and no listener is notified.
    fillSequential(MAX_OBSERVER_EVENTS + 1);
    const before = getAgentObserverSnapshot(AGENT_PUBKEY).events;
    assert.equal(before.length, OBSERVER_EVENTS_LOW_WATER);
    const beforeFirstSeq = before.at(0).seq;

    let notifications = 0;
    const unsubscribe = subscribeAgentObserverStore(() => {
      notifications += 1;
    });
    try {
      // seq 1..(beforeFirstSeq - 1) were discarded; replay a spread of them
      // plus the boundary event itself (beforeFirstSeq - 1 is the floor).
      for (let seq = 1; seq < beforeFirstSeq; seq += 1) {
        syncAgentObserverEvents(AGENT_PUBKEY, [makeEvent(seq)]);
      }
    } finally {
      unsubscribe();
    }

    const after = getAgentObserverSnapshot(AGENT_PUBKEY).events;
    assert.deepEqual(
      after,
      before,
      "replaying already-evicted frames must not change the retained window",
    );
    assert.equal(
      notifications,
      0,
      "a stale-only replay does no work and notifies no listener",
    );
  });

  it("test_pre_floor_frame_after_refill_drops_no_retained_events", () => {
    // Overflow to low-water, refill to the cap through ordinary appends, then
    // inject a single pre-floor (already-evicted) frame. Pre-fix this pushed
    // length to cap+1 and trimmed away 300 legitimate retained events; the
    // floor now rejects it, so the retained window is untouched.
    fillSequential(MAX_OBSERVER_EVENTS + 1);
    const floorBoundarySeq =
      getAgentObserverSnapshot(AGENT_PUBKEY).events.at(0).seq;
    const headroom = MAX_OBSERVER_EVENTS - OBSERVER_EVENTS_LOW_WATER;
    for (let i = 1; i <= headroom; i += 1) {
      syncAgentObserverEvents(AGENT_PUBKEY, [
        makeEvent(MAX_OBSERVER_EVENTS + 1 + i),
      ]);
    }
    const atCap = getAgentObserverSnapshot(AGENT_PUBKEY).events;
    assert.equal(atCap.length, MAX_OBSERVER_EVENTS, "refilled back to the cap");

    // A pre-floor frame (seq strictly below the boundary that was evicted).
    syncAgentObserverEvents(AGENT_PUBKEY, [makeEvent(floorBoundarySeq - 1)]);

    const after = getAgentObserverSnapshot(AGENT_PUBKEY).events;
    assert.equal(
      after.length,
      MAX_OBSERVER_EVENTS,
      "a rejected pre-floor frame must not trigger a trim below the cap",
    );
    assert.deepEqual(
      after,
      atCap,
      "no legitimate retained event is dropped by a pre-floor arrival",
    );
  });

  it("test_out_of_order_frame_newer_than_floor_is_still_admitted", () => {
    // The floor must reject only already-evicted history, never a legitimate
    // out-of-order frame that sorts after the floor. Such a frame lands in the
    // retained window (via the rebuild fallback) and advances the length.
    fillSequential(MAX_OBSERVER_EVENTS + 1);
    const retained = getAgentObserverSnapshot(AGENT_PUBKEY).events;
    const oldestRetainedSeq = retained.at(0).seq;
    const lengthBefore = retained.length;

    // The eviction floor is the boundary event just below the retained window
    // (seq oldestRetainedSeq - 1). Construct a never-seen frame whose timestamp
    // sits strictly between the floor and the oldest retained event — out of
    // order versus the tail, but newer than the floor, so it must be admitted.
    const floorSeq = oldestRetainedSeq - 1;
    const oooEvent = {
      seq: 1_000_000_000,
      timestamp: new Date(
        1_760_000_000_000 + floorSeq * 1000 + 500,
      ).toISOString(),
      kind: "turn_started",
      agentIndex: 0,
      channelId: "chan-1",
      sessionId: "sess-1",
      turnId: "turn-ooo",
      payload: {},
    };
    syncAgentObserverEvents(AGENT_PUBKEY, [oooEvent]);

    const after = getAgentObserverSnapshot(AGENT_PUBKEY).events;
    assert.equal(
      after.length,
      lengthBefore + 1,
      "an out-of-order frame newer than the floor is admitted, not rejected",
    );
    assert.ok(
      after.some((event) => event.seq === oooEvent.seq),
      "the admitted frame is present in the retained window",
    );
  });
});

describe("live observer journal — in-order append fast path ordering/dedup", () => {
  // The common live path (every batch strictly after the retained tail) skips
  // the whole-journal dedup Set and re-sort. These pin the observable
  // invariants that authorize that skip: identical ordering, dedup, and
  // transcript against the general path.
  beforeEach(() => {
    resetAgentObserverStore();
  });

  /** An event with an explicit timestamp, for equal-timestamp tie-breaks. */
  function makeEventAt(seq, timestampMs) {
    return {
      ...makeEvent(seq),
      timestamp: new Date(timestampMs).toISOString(),
    };
  }

  it("test_equal_timestamp_batch_orders_by_seq", () => {
    // A one-second harness frame batches several events sharing a timestamp;
    // the tie-break is seq. Deliver them out of seq order in one batch.
    const t = 1_760_000_100_000;
    syncAgentObserverEvents(AGENT_PUBKEY, [makeEventAt(1, t - 1000)]);
    syncAgentObserverEvents(AGENT_PUBKEY, [
      makeEventAt(4, t),
      makeEventAt(2, t),
      makeEventAt(3, t),
    ]);
    assert.deepEqual(
      getAgentObserverSnapshot(AGENT_PUBKEY).events.map((event) => event.seq),
      [1, 2, 3, 4],
      "equal-timestamp events are retained in seq order",
    );
  });

  it("test_duplicate_batch_redelivery_is_ignored", () => {
    // Relay redelivery of the newest batch: every event duplicates the tail,
    // so nothing is admitted, nothing is notified.
    const batch = [makeEvent(1), makeEvent(2), makeEvent(3)];
    syncAgentObserverEvents(AGENT_PUBKEY, batch);
    let notifications = 0;
    const unsubscribe = subscribeAgentObserverStore(() => {
      notifications += 1;
    });
    try {
      syncAgentObserverEvents(AGENT_PUBKEY, batch);
    } finally {
      unsubscribe();
    }
    assert.deepEqual(
      getAgentObserverSnapshot(AGENT_PUBKEY).events.map((event) => event.seq),
      [1, 2, 3],
      "a redelivered batch adds nothing",
    );
    assert.equal(notifications, 0, "a pure-duplicate batch notifies no one");
  });

  it("test_batch_with_intra_batch_duplicate_admits_once", () => {
    // A batch strictly after the tail still dedups within itself.
    syncAgentObserverEvents(AGENT_PUBKEY, [makeEvent(1)]);
    syncAgentObserverEvents(AGENT_PUBKEY, [
      makeEvent(2),
      makeEvent(3),
      makeEvent(2),
    ]);
    assert.deepEqual(
      getAgentObserverSnapshot(AGENT_PUBKEY).events.map((event) => event.seq),
      [1, 2, 3],
      "an intra-batch duplicate is admitted exactly once",
    );
  });

  it("test_late_arrival_overlapping_tail_takes_slow_path_and_dedups", () => {
    // A replayed window straddling the tail: partly duplicate, partly new,
    // partly older-than-tail. Not all-after, so the general path must dedup
    // against the whole journal and re-sort.
    syncAgentObserverEvents(AGENT_PUBKEY, [
      makeEvent(1),
      makeEvent(2),
      makeEvent(4),
    ]);
    syncAgentObserverEvents(AGENT_PUBKEY, [
      makeEvent(2),
      makeEvent(3),
      makeEvent(4),
      makeEvent(5),
    ]);
    const events = getAgentObserverSnapshot(AGENT_PUBKEY).events;
    assert.deepEqual(
      events.map((event) => event.seq),
      [1, 2, 3, 4, 5],
      "overlapping late arrival dedups against the journal and sorts into place",
    );
    assert.deepEqual(
      getAgentTranscript(AGENT_PUBKEY),
      buildTranscript(events),
      "the transcript equals a full replay after a mixed-path sequence",
    );
  });

  it("test_fast_path_transcript_equals_full_replay", () => {
    // Pure in-order streaming (fast path every time) must produce the same
    // derived transcript as a replay of the retained window.
    fillSequential(50);
    const events = getAgentObserverSnapshot(AGENT_PUBKEY).events;
    assert.equal(events.length, 50);
    assert.deepEqual(
      getAgentTranscript(AGENT_PUBKEY),
      buildTranscript(events),
      "in-order fast-path appends derive the same transcript as a replay",
    );
  });
});
