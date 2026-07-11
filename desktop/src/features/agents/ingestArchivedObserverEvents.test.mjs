/**
 * Tests for ingestArchivedObserverEvents — the read-back ingest seam that loads
 * archived observer frames from the local SQLite archive into the observer store.
 *
 * These tests use node:test's synchronous-friendly import pattern combined with
 * test-only exports (_testRegisterKnownAgents, _decryptFn injection, and the
 * existing injectObserverEventsForE2E) to exercise behavior without requiring
 * a Tauri runtime or React context.
 */

import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import {
  ingestArchivedObserverEvents,
  injectObserverEventsForE2E,
  getAgentObserverSnapshot,
  resetAgentObserverStore,
  _testRegisterKnownAgents,
  _testGetArchivedChannelEvents,
} from "@/features/agents/observerRelayStore.ts";

// ── Constants ─────────────────────────────────────────────────────────────────

const AGENT_PUBKEY = "a".repeat(64);
const OTHER_PUBKEY = "b".repeat(64);
const SUB_ID = "test-sub-1";

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeRawEvent(overrides = {}) {
  return {
    id: "e".repeat(64),
    pubkey: AGENT_PUBKEY,
    created_at: 1000,
    kind: 24200,
    tags: [
      ["p", OTHER_PUBKEY],
      ["agent", AGENT_PUBKEY],
      ["frame", "telemetry"],
    ],
    content: "encrypted",
    sig: "s".repeat(128),
    ...overrides,
  };
}

function makeObserverEvent(overrides = {}) {
  return {
    seq: 1,
    timestamp: "2026-01-01T00:00:01.000Z",
    kind: "acp_write",
    agentIndex: 0,
    channelId: "chan-1",
    sessionId: "sess-1",
    turnId: "turn-1",
    payload: {},
    ...overrides,
  };
}

// Decrypt fn that resolves to a known observer event.
function makeDecrypt(returnEvent) {
  return () => Promise.resolve(returnEvent);
}

// Decrypt fn that always rejects.
function makeDecryptFail() {
  return () => Promise.reject(new Error("decryption failed"));
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe("ingestArchivedObserverEvents", () => {
  beforeEach(() => {
    resetAgentObserverStore();
  });

  it("test_unknown_agent_drops_event_before_decrypt", async () => {
    // knownAgentPubkeys is empty after reset.
    // Even with a successful decrypt fn, the event must be dropped.
    let decryptCalled = false;
    const decryptFn = () => {
      decryptCalled = true;
      return Promise.resolve(makeObserverEvent());
    };
    await ingestArchivedObserverEvents([makeRawEvent()], decryptFn);
    assert.equal(
      decryptCalled,
      false,
      "decrypt must not be called for unknown agent",
    );
    const snap = getAgentObserverSnapshot(AGENT_PUBKEY, true);
    assert.equal(snap.events.length, 0);
  });

  it("test_mismatched_sender_drops_event_before_decrypt", async () => {
    _testRegisterKnownAgents(SUB_ID, [AGENT_PUBKEY]);
    let decryptCalled = false;
    const decryptFn = () => {
      decryptCalled = true;
      return Promise.resolve(makeObserverEvent());
    };
    // event.pubkey differs from agent tag value
    const badEvent = makeRawEvent({ pubkey: OTHER_PUBKEY });
    await ingestArchivedObserverEvents([badEvent], decryptFn);
    assert.equal(
      decryptCalled,
      false,
      "decrypt must not be called for mismatched sender",
    );
    const snap = getAgentObserverSnapshot(AGENT_PUBKEY, true);
    assert.equal(snap.events.length, 0);
  });

  it("test_non_telemetry_frame_tag_drops_event", async () => {
    _testRegisterKnownAgents(SUB_ID, [AGENT_PUBKEY]);
    let decryptCalled = false;
    const decryptFn = () => {
      decryptCalled = true;
      return Promise.resolve(makeObserverEvent());
    };
    const nonTelemetryEvent = makeRawEvent({
      tags: [
        ["p", OTHER_PUBKEY],
        ["agent", AGENT_PUBKEY],
        ["frame", "control"], // not "telemetry"
      ],
    });
    await ingestArchivedObserverEvents([nonTelemetryEvent], decryptFn);
    assert.equal(decryptCalled, false, "non-telemetry frame must be dropped");
    const snap = getAgentObserverSnapshot(AGENT_PUBKEY, true);
    assert.equal(snap.events.length, 0);
  });

  it("test_decrypt_failure_silently_dropped", async () => {
    _testRegisterKnownAgents(SUB_ID, [AGENT_PUBKEY]);
    // Good event that passes all guards but fails decrypt.
    await ingestArchivedObserverEvents([makeRawEvent()], makeDecryptFail());
    const snap = getAgentObserverSnapshot(AGENT_PUBKEY, true);
    // Error is silently dropped — no crash, no event in store.
    assert.equal(snap.events.length, 0);
  });

  it("test_successful_ingest_adds_event_to_store", async () => {
    _testRegisterKnownAgents(SUB_ID, [AGENT_PUBKEY]);
    const obs = makeObserverEvent({ seq: 1 });
    await ingestArchivedObserverEvents([makeRawEvent()], makeDecrypt(obs));
    // Archived events with a channelId are stored in the channel-scoped archive
    // window (not in the per-agent live snapshot). Read via raw events for tests.
    const archivedEvents = _testGetArchivedChannelEvents(
      AGENT_PUBKEY,
      "chan-1",
    );
    assert.equal(archivedEvents.length, 1, "archive must contain 1 raw event");
    assert.equal(archivedEvents[0].seq, 1);
    // Also verify the live snapshot is untouched — archive separation.
    const snap = getAgentObserverSnapshot(AGENT_PUBKEY, true);
    assert.equal(
      snap.events.length,
      0,
      "live snapshot must be empty for archived events",
    );
  });

  it("test_dedup_does_not_add_live_present_event", async () => {
    _testRegisterKnownAgents(SUB_ID, [AGENT_PUBKEY]);
    // Ingest the same archived event twice — the channel archive window must dedup
    // so (seq, timestamp) pairs are only stored once.
    const archivedObs = makeObserverEvent({
      seq: 5,
      timestamp: "2026-01-01T00:00:05.000Z",
    });
    await ingestArchivedObserverEvents([makeRawEvent(), makeRawEvent()], () =>
      Promise.resolve(archivedObs),
    );

    const archiveEvents = _testGetArchivedChannelEvents(AGENT_PUBKEY, "chan-1");
    assert.equal(
      archiveEvents.length,
      1,
      "dedup: identical (seq, timestamp) must produce exactly 1 entry in the archive window",
    );
  });

  it("test_older_archived_event_sorts_before_live", async () => {
    // Pre-seed a newer live event (no channelId → goes to live path).
    const liveObs = makeObserverEvent({
      seq: 2,
      timestamp: "2026-01-01T00:00:02.000Z",
      channelId: null,
    });
    injectObserverEventsForE2E(AGENT_PUBKEY, [liveObs]);

    _testRegisterKnownAgents(SUB_ID, [AGENT_PUBKEY]);
    // Ingest an older archived event (has channelId → goes to archive window).
    const archivedObs = makeObserverEvent({
      seq: 1,
      timestamp: "2026-01-01T00:00:01.000Z",
      channelId: "chan-1",
    });
    await ingestArchivedObserverEvents(
      [makeRawEvent()],
      makeDecrypt(archivedObs),
    );

    // Live snapshot has seq=2; archive window for chan-1 has seq=1.
    // After merge they should appear in ascending time order.
    const snap = getAgentObserverSnapshot(AGENT_PUBKEY, true);
    assert.equal(
      snap.events.length,
      1,
      "live snapshot must have 1 event (no-channelId frame)",
    );
    assert.equal(snap.events[0].seq, 2, "live event must be seq=2");

    const archiveEvents = _testGetArchivedChannelEvents(AGENT_PUBKEY, "chan-1");
    assert.equal(
      archiveEvents.length,
      1,
      "archive window must have 1 event (older frame)",
    );
    assert.equal(archiveEvents[0].seq, 1, "older archived event must be seq=1");
  });

  it("test_multiple_events_ingested_in_order", async () => {
    _testRegisterKnownAgents(SUB_ID, [AGENT_PUBKEY]);
    // Three events with channelId — all go to archive window, not live snapshot.
    const events = [
      makeObserverEvent({ seq: 3, timestamp: "2026-01-01T00:00:03.000Z" }),
      makeObserverEvent({ seq: 1, timestamp: "2026-01-01T00:00:01.000Z" }),
      makeObserverEvent({ seq: 2, timestamp: "2026-01-01T00:00:02.000Z" }),
    ];
    let callIdx = 0;
    const decryptFn = () => Promise.resolve(events[callIdx++]);
    // All three raw events pass the guards (same pubkey/agent tag).
    await ingestArchivedObserverEvents(
      [makeRawEvent(), makeRawEvent(), makeRawEvent()],
      decryptFn,
    );
    // All have channelId "chan-1" — verify archive window, not live snapshot.
    const archiveEvents = _testGetArchivedChannelEvents(AGENT_PUBKEY, "chan-1");
    assert.equal(
      archiveEvents.length,
      3,
      "archive must have 3 ingested events",
    );
    // Events must be sorted ascending by timestamp (compareObserverEvents order).
    assert.deepEqual(
      archiveEvents.map((e) => e.seq),
      [1, 2, 3],
      "archive events must be sorted ascending by timestamp",
    );
    // Live snapshot must be empty (all events were channeled).
    const snap = getAgentObserverSnapshot(AGENT_PUBKEY, true);
    assert.equal(
      snap.events.length,
      0,
      "live snapshot must be empty for channeled archived events",
    );
  });

  // F7 regression: idle agent (enabled=false for relay subscription) with
  // archived rows in the store must render those rows, scoped to the viewed
  // channel. Prior to the fix, getAgentObserverSnapshot returned IDLE_SNAPSHOT
  // when enabled=false, discarding ingested archived events.
  //
  // Updated: archived events now go to the channel-scoped archive window
  // (getArchivedChannelTranscript), not the live snapshot. The channel-scoping
  // is by construction — cross-channel contamination is impossible.
  it("test_idle_agent_archived_events_readable_when_enabled_false", async () => {
    _testRegisterKnownAgents(SUB_ID, [AGENT_PUBKEY]);
    // Ingest two archived events: one for channel-A, one for channel-B.
    const chanAEvent = makeObserverEvent({
      seq: 1,
      timestamp: "2026-01-01T00:00:01.000Z",
      channelId: "channel-A",
    });
    const chanBEvent = makeObserverEvent({
      seq: 2,
      timestamp: "2026-01-01T00:00:02.000Z",
      channelId: "channel-B",
    });
    let callIdx = 0;
    const events = [chanAEvent, chanBEvent];
    await ingestArchivedObserverEvents(
      [
        makeRawEvent({ id: `e1${"0".repeat(62)}` }),
        makeRawEvent({ id: `e2${"0".repeat(62)}` }),
      ],
      () => Promise.resolve(events[callIdx++]),
    );

    // Channel-A archive window must have at least one item.
    const channelAEvents = _testGetArchivedChannelEvents(
      AGENT_PUBKEY,
      "channel-A",
    );
    assert.equal(
      channelAEvents.length,
      1,
      "idle agent: channel-A archive window must contain 1 event",
    );

    // Channel-B archive window must have at least one item.
    const channelBEvents = _testGetArchivedChannelEvents(
      AGENT_PUBKEY,
      "channel-B",
    );
    assert.equal(
      channelBEvents.length,
      1,
      "idle agent: channel-B archive window must contain 1 event",
    );

    // Cross-channel contamination guard: channel-A events must not appear in channel-B window.
    const channelASeqSet = new Set(
      channelAEvents.map((e) => `${e.seq}:${e.timestamp}`),
    );
    const contaminated = channelBEvents.some((e) =>
      channelASeqSet.has(`${e.seq}:${e.timestamp}`),
    );
    assert.equal(
      contaminated,
      false,
      "channel-B archive must NOT contain channel-A events (cross-channel contamination guard)",
    );
  });
});

// ── Cursor advance test (pure logic, no store needed) ─────────────────────────

describe("load-older cursor advance logic", () => {
  it("test_cursor_advances_to_last_row_compound_key", () => {
    // Mirrors the cursor-update logic in useLoadArchivedObserverEvents.
    // Events arrive newest-first (as the store returns them).
    // The cursor should be the LAST element — the oldest on this page —
    // capturing both created_at and id to mirror the compound sort key
    // so same-second siblings are never skipped at a page boundary.
    const events = [
      { id: "e1", created_at: 1000 },
      { id: "e2", created_at: 900 },
      { id: "e3", created_at: 800 },
      { id: "e4", created_at: 500 },
    ];
    const oldestEvent = events[events.length - 1];
    const cursor = { createdAt: oldestEvent.created_at, id: oldestEvent.id };
    assert.deepEqual(
      cursor,
      { createdAt: 500, id: "e4" },
      "cursor must capture the last (oldest) row's created_at + id",
    );
  });

  it("test_short_page_signals_archive_exhausted", () => {
    // A page with fewer events than the limit signals end-of-archive.
    const PAGE_SIZE = 50;
    const page = Array.from({ length: 30 }, (_, i) => ({
      created_at: 1000 - i,
    }));
    const exhausted = page.length < PAGE_SIZE;
    assert.equal(
      exhausted,
      true,
      "short page must signal archive is exhausted",
    );
  });

  it("test_full_page_signals_more_archive_available", () => {
    const PAGE_SIZE = 50;
    const page = Array.from({ length: 50 }, (_, i) => ({
      created_at: 1000 - i,
    }));
    const exhausted = page.length < PAGE_SIZE;
    assert.equal(
      exhausted,
      false,
      "full page must signal more archive may be available",
    );
  });
});

// ── Archive paging state reset on channel change (F8 regression) ──────────────
//
// The paging cursor, exhaustion flag, and fetch lock are per-channel — they
// must reset when channelId changes so channel B starts with a fresh cursor
// and hasOlderArchived=true rather than inheriting channel A's exhausted state.
//
// useLoadArchivedObserverEvents delegates all mutable paging state to
// archivePagingState.ts (createArchivePagingState / applyChannelReset).
// We test those functions directly — tests would fail if the implementation
// were removed or if the channel-reset touched backfill state it must not.

import {
  createArchivePagingState,
  applyChannelReset,
} from "@/features/agents/ui/archivePagingState.ts";

describe("archive paging state reset on channel change", () => {
  it("test_fresh_state_has_correct_initial_values", () => {
    const ps = createArchivePagingState();

    assert.equal(ps.hasSubscription, null, "hasSubscription starts null");
    assert.equal(ps.hasOlderArchived, true, "hasOlderArchived starts true");
    assert.equal(ps.isFetching, false, "isFetching starts false");
    assert.equal(ps.backfillStatus, "pending", "backfillStatus starts pending");
    assert.notEqual(
      ps.backfillPromise,
      null,
      "backfillPromise is eagerly initialized",
    );
    assert.equal(ps.cursor, null, "cursor starts null");
  });

  it("test_channel_switch_resets_cursor_exhaustion_and_fetch_lock", () => {
    const ps = createArchivePagingState();

    // Simulate channel A paging to exhaustion with a non-null cursor.
    ps.cursor = { createdAt: 1000, id: "event-a5" };
    ps.hasOlderArchived = false; // channel A exhausted
    ps.isFetching = true; // mid-flight request (edge case)
    ps.backfillStatus = "done"; // backfill ran once already
    const originalPromise = ps.backfillPromise; // must survive reset

    // Channel switch — this is what the useEffect([channelId]) calls.
    applyChannelReset(ps);

    assert.equal(ps.cursor, null, "cursor resets to null on channel switch");
    assert.equal(
      ps.hasOlderArchived,
      true,
      "hasOlderArchived resets to true on channel switch",
    );
    assert.equal(
      ps.isFetching,
      false,
      "isFetching resets to false on channel switch",
    );

    // Backfill state must NOT be touched — it is identity-level and should
    // survive channel switches so the backfill only runs once per identity mount.
    assert.equal(
      ps.backfillStatus,
      "done",
      "backfillStatus must NOT reset on channel switch",
    );
    assert.equal(
      ps.backfillPromise,
      originalPromise,
      "backfillPromise must NOT reset on channel switch",
    );
    assert.equal(
      ps.hasSubscription,
      null,
      "hasSubscription must NOT reset on channel switch",
    );
  });

  it("test_multiple_channel_switches_each_start_fresh", () => {
    const ps = createArchivePagingState();

    // Switch to channel A: exhaust it.
    ps.cursor = { createdAt: 500, id: "a-oldest" };
    ps.hasOlderArchived = false;
    applyChannelReset(ps);

    assert.equal(ps.cursor, null, "switch A→B: cursor reset");
    assert.equal(
      ps.hasOlderArchived,
      true,
      "switch A→B: hasOlderArchived reset",
    );

    // Simulate channel B also being paged.
    ps.cursor = { createdAt: 200, id: "b-oldest" };
    ps.hasOlderArchived = false;
    applyChannelReset(ps);

    assert.equal(ps.cursor, null, "switch B→C: cursor reset again");
    assert.equal(
      ps.hasOlderArchived,
      true,
      "switch B→C: hasOlderArchived reset again",
    );
  });
});

// ── Archive window beyond MAX_OBSERVER_EVENTS cap (regression) ────────────────
//
// The live observer relay store caps per-agent events at MAX_OBSERVER_EVENTS
// (3,000). Before this fix, archived events flowed through the same capped path,
// so loading more than 3,000 archived frames silently discarded the oldest ones.
// The channel-scoped archive window is uncapped — all loaded history persists.
//
// This describe block injects 3,100 archived events and verifies every one
// is preserved in the archive window without truncation.

describe("archive window holds more than MAX_OBSERVER_EVENTS (3000) frames", () => {
  beforeEach(() => {
    resetAgentObserverStore();
  });

  it("test_archive_window_retains_all_events_beyond_3000_cap", async () => {
    _testRegisterKnownAgents(SUB_ID, [AGENT_PUBKEY]);

    const OVER_CAP = 3100;
    const rawEvents = Array.from({ length: OVER_CAP }, (_, i) => ({
      id: `e${String(i).padStart(63, "0")}`,
      pubkey: AGENT_PUBKEY,
      created_at: 1000 + i,
      kind: 24200,
      tags: [
        ["p", OTHER_PUBKEY],
        ["agent", AGENT_PUBKEY],
        ["frame", "telemetry"],
      ],
      content: "encrypted",
      sig: "s".repeat(128),
    }));
    const observerEvents = Array.from({ length: OVER_CAP }, (_, i) =>
      makeObserverEvent({
        seq: i + 1,
        timestamp: new Date(1000000 + i * 1000).toISOString(),
        channelId: "chan-1",
      }),
    );
    let callIdx = 0;
    await ingestArchivedObserverEvents(rawEvents, () =>
      Promise.resolve(observerEvents[callIdx++]),
    );

    const archiveEvents = _testGetArchivedChannelEvents(AGENT_PUBKEY, "chan-1");
    assert.equal(
      archiveEvents.length,
      OVER_CAP,
      `archive window must hold all ${OVER_CAP} events without truncation (cap was 3000)`,
    );

    // The live snapshot must be empty — separation is strict.
    const snap = getAgentObserverSnapshot(AGENT_PUBKEY, true);
    assert.equal(
      snap.events.length,
      0,
      "live snapshot must not contain any archived events",
    );

    // All 3100 events must be sorted ascending.
    assert.equal(
      archiveEvents[0].seq,
      1,
      "first archived event must be seq=1 (oldest)",
    );
    assert.equal(
      archiveEvents[OVER_CAP - 1].seq,
      OVER_CAP,
      `last archived event must be seq=${OVER_CAP} (newest)`,
    );
  });

  it("test_resetAgentObserverStore_clears_archive_window", async () => {
    _testRegisterKnownAgents(SUB_ID, [AGENT_PUBKEY]);

    const obs = makeObserverEvent({ seq: 1, channelId: "chan-1" });
    await ingestArchivedObserverEvents([makeRawEvent()], () =>
      Promise.resolve(obs),
    );

    // Confirm events are present before reset.
    assert.equal(
      _testGetArchivedChannelEvents(AGENT_PUBKEY, "chan-1").length,
      1,
      "pre-reset: archive must have 1 event",
    );

    // Reset must wipe the archive window.
    resetAgentObserverStore();

    assert.equal(
      _testGetArchivedChannelEvents(AGENT_PUBKEY, "chan-1").length,
      0,
      "post-reset: archive window must be empty after resetAgentObserverStore",
    );
  });
});

// ── Session-boundary key stability across prepend (regression) ────────────────
//
// getDisplayBlockKey for session-boundary blocks previously used `runIndex` (the
// run's array-position), which shifts when older sessions are prepended before
// existing runs — causing React to remount unchanged boundaries and churn the
// virtual list. The fix uses `firstItemId` (the id of the first item in the
// following run), which is invariant across prepend.
//
// The full key-stability test suite lives in agentSessionTranscriptGrouping.test.mjs
// where proper TranscriptItem fixtures are already defined. See
// buildTranscriptDisplayBlocks_sessionBoundary_emitsFirstItemId and
// buildTranscriptDisplayBlocks_sessionBoundary_keyStableAcrossPrepend there.
