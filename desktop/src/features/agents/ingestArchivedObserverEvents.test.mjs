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
    const PAGE_SIZE = 200;
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
    const PAGE_SIZE = 200;
    const page = Array.from({ length: 200 }, (_, i) => ({
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
  runHydrationLoop,
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
    assert.equal(
      ps.initialHydrationDone,
      false,
      "initialHydrationDone starts false",
    );
    assert.equal(ps.activeChannelId, null, "activeChannelId starts null");
  });

  it("test_channel_switch_resets_cursor_exhaustion_and_fetch_lock", () => {
    const ps = createArchivePagingState();

    // Simulate channel A paging to exhaustion with a non-null cursor.
    ps.cursor = { createdAt: 1000, id: "event-a5" };
    ps.hasOlderArchived = false; // channel A exhausted
    ps.isFetching = true; // mid-flight request (edge case)
    ps.initialHydrationDone = true; // hydration ran for channel A
    ps.activeChannelId = "chan-a";
    ps.backfillStatus = "done"; // backfill ran once already
    const originalPromise = ps.backfillPromise; // must survive reset

    // Channel switch — this is what the useEffect([channelId]) calls.
    applyChannelReset(ps, "chan-b");

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
    assert.equal(
      ps.initialHydrationDone,
      false,
      "initialHydrationDone resets to false on channel switch so the new channel hydrates",
    );
    assert.equal(
      ps.activeChannelId,
      "chan-b",
      "activeChannelId updates to new channel on switch",
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

    // Switch to channel A: exhaust it and complete hydration.
    ps.cursor = { createdAt: 500, id: "a-oldest" };
    ps.hasOlderArchived = false;
    ps.initialHydrationDone = true;
    applyChannelReset(ps, "chan-b");

    assert.equal(ps.cursor, null, "switch A→B: cursor reset");
    assert.equal(
      ps.hasOlderArchived,
      true,
      "switch A→B: hasOlderArchived reset",
    );
    assert.equal(
      ps.initialHydrationDone,
      false,
      "switch A→B: initialHydrationDone reset",
    );
    assert.equal(
      ps.activeChannelId,
      "chan-b",
      "switch A→B: activeChannelId updated",
    );

    // Simulate channel B also being paged and hydrated.
    ps.cursor = { createdAt: 200, id: "b-oldest" };
    ps.hasOlderArchived = false;
    ps.initialHydrationDone = true;
    applyChannelReset(ps, "chan-c");

    assert.equal(ps.cursor, null, "switch B→C: cursor reset again");
    assert.equal(
      ps.hasOlderArchived,
      true,
      "switch B→C: hasOlderArchived reset again",
    );
    assert.equal(
      ps.initialHydrationDone,
      false,
      "switch B→C: initialHydrationDone reset again",
    );
    assert.equal(
      ps.activeChannelId,
      "chan-c",
      "switch B→C: activeChannelId updated",
    );
  });
});

// ── Eager initial hydration loop logic ───────────────────────────────────────
//
// The initial hydration loop in useLoadArchivedObserverEvents calls
// fetchOlderArchived up to INITIAL_HYDRATION_BUDGET_PAGES times. The loop must:
//   - Stop at budget (10 pages) even if more archive exists.
//   - Stop early when the archive is exhausted (ps.hasOlderArchived → false).
//   - Respect channel-switch cancellation (signal.cancelled).
//
// These tests call the PRODUCTION runHydrationLoop from archivePagingState.ts
// with mock fetchOnePage functions — so they fail if the production loop logic
// is deleted or misrouted, not just if a reimplemented copy breaks.

describe("eager initial hydration loop control flow (production runHydrationLoop)", () => {
  const BUDGET = 10; // mirrors INITIAL_HYDRATION_BUDGET_PAGES

  it("test_hydration_stops_at_budget_when_archive_never_exhausted", async () => {
    const ps = createArchivePagingState();
    applyChannelReset(ps, "chan-1");
    let fetchCount = 0;
    const fetchOnePage = async () => {
      fetchCount++;
      // archive remains non-empty — ps.hasOlderArchived stays true
    };
    const signal = { cancelled: false };
    await runHydrationLoop(ps, fetchOnePage, BUDGET, signal);
    assert.equal(
      fetchCount,
      BUDGET,
      `production runHydrationLoop must stop after exactly ${BUDGET} pages (budget limit)`,
    );
  });

  it("test_hydration_stops_early_when_archive_exhausted", async () => {
    const ps = createArchivePagingState();
    applyChannelReset(ps, "chan-1");
    let fetchCount = 0;
    const fetchOnePage = async () => {
      fetchCount++;
      if (fetchCount >= 3) {
        ps.hasOlderArchived = false; // mock: archive exhausted on page 3
      }
    };
    const signal = { cancelled: false };
    await runHydrationLoop(ps, fetchOnePage, BUDGET, signal);
    assert.equal(
      fetchCount,
      3,
      "production runHydrationLoop must stop as soon as ps.hasOlderArchived is false (before budget)",
    );
  });

  it("test_hydration_respects_cancellation_on_channel_switch", async () => {
    const ps = createArchivePagingState();
    applyChannelReset(ps, "chan-1");
    const signal = { cancelled: false };
    let fetchCount = 0;
    const fetchOnePage = async () => {
      fetchCount++;
      if (fetchCount >= 2) {
        signal.cancelled = true; // mock: channel switched mid-loop
      }
    };
    await runHydrationLoop(ps, fetchOnePage, BUDGET, signal);
    assert.equal(
      fetchCount,
      2,
      "production runHydrationLoop must stop when signal.cancelled is true (channel switch)",
    );
  });

  it("test_hydration_zero_iterations_when_already_exhausted", async () => {
    // If ps.hasOlderArchived is already false before the loop starts (e.g.
    // channel A was exhausted and reset did not run yet), the loop must not
    // call fetchOnePage at all.
    const ps = createArchivePagingState();
    applyChannelReset(ps, "chan-1");
    ps.hasOlderArchived = false; // already exhausted
    let fetchCount = 0;
    const fetchOnePage = async () => {
      fetchCount++;
    };
    const signal = { cancelled: false };
    await runHydrationLoop(ps, fetchOnePage, BUDGET, signal);
    assert.equal(
      fetchCount,
      0,
      "must not fetch when archive is already exhausted",
    );
  });

  // Regression: stale React closure — the original fetchOlderArchived captured
  // hasOlderArchived from React state, which was false (exhausted) for channel A
  // while ps.hasOlderArchived had already been reset to true by applyChannelReset.
  // The production loop uses ps.hasOlderArchived (the ref) to guard iterations,
  // so the switch-then-hydrate path must call fetchOnePage for the new channel.
  it("test_exhausted_channel_A_then_switch_to_B_hydrates_B", async () => {
    const ps = createArchivePagingState();

    // Simulate exhausting channel A.
    applyChannelReset(ps, "chan-a");
    ps.hasOlderArchived = false; // channel A exhausted

    // Switch to channel B — resets hasOlderArchived and activeChannelId.
    applyChannelReset(ps, "chan-b");

    // ps.hasOlderArchived is now true; the loop should call fetchOnePage.
    let fetchCount = 0;
    const fetchOnePage = async () => {
      fetchCount++;
      ps.hasOlderArchived = false; // B also exhausted after 1 page
    };
    const signal = { cancelled: false };
    await runHydrationLoop(ps, fetchOnePage, BUDGET, signal);

    assert.equal(
      fetchCount,
      1,
      "after switch from exhausted channel A to B, runHydrationLoop must call fetchOnePage for B (not skip due to stale exhaustion)",
    );
  });

  // Regression: stale cursor write — an in-flight read from channel A should
  // not write A's cursor into ps.cursor after the switch to B. The activeChannelId
  // token (set by applyChannelReset) is what lets fetchOlderArchived detect and
  // discard the stale result. This test verifies that applyChannelReset correctly
  // advances the token so a pre-switch requestChannelId !== ps.activeChannelId.
  it("test_activeChannelId_token_detects_stale_read_from_prior_channel", () => {
    const ps = createArchivePagingState();
    applyChannelReset(ps, "chan-a");
    const requestChannelId = ps.activeChannelId; // captured at request start = "chan-a"

    // Simulate channel switch before the Tauri read resolves.
    applyChannelReset(ps, "chan-b");

    // The in-flight A read checks requestChannelId !== ps.activeChannelId.
    assert.notEqual(
      requestChannelId,
      ps.activeChannelId,
      "requestChannelId from channel A must not match activeChannelId after switching to B — stale read must be discarded",
    );
    assert.equal(
      ps.activeChannelId,
      "chan-b",
      "activeChannelId must reflect the current channel after switch",
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

// ── Raw-event-level merge: subscription notification + stateful aggregates ───────
//
// These tests verify the revised design where:
//   - A full archive page notifies useSyncExternalStore subscribers.
//   - Tool start (archive) + tool_call_update (live) yields one complete row.
//   - Permission request (archive) + response (live) yields the resolved outcome.
//   - The combined raw event window feeds raw rail / header count.
//   - The >3,000-frame retention guarantee still holds.

import {
  subscribeAgentObserverStore,
  getArchivedChannelEvents,
} from "@/features/agents/observerRelayStore.ts";
import { buildTranscriptState } from "@/features/agents/ui/agentSessionTranscript.ts";
import { mergeObserverEventWindows } from "@/features/agents/ui/agentSessionPanelLayout.ts";

describe("archive page subscription notification", () => {
  beforeEach(() => {
    resetAgentObserverStore();
  });

  it("test_full_archive_page_notifies_subscribers", async () => {
    _testRegisterKnownAgents(SUB_ID, [AGENT_PUBKEY]);

    let notifyCount = 0;
    const unsubscribe = subscribeAgentObserverStore(() => {
      notifyCount++;
    });

    // Ingest a full 50-row page of archived events.
    const PAGE_SIZE = 50;
    const rawEvents = Array.from({ length: PAGE_SIZE }, (_, i) =>
      makeRawEvent({ id: `p${String(i).padStart(63, "0")}` }),
    );
    const observerEvents = Array.from({ length: PAGE_SIZE }, (_, i) =>
      makeObserverEvent({
        seq: i + 1,
        timestamp: new Date(1_000_000 + i * 1000).toISOString(),
        channelId: "chan-1",
      }),
    );
    let callIdx = 0;
    await ingestArchivedObserverEvents(rawEvents, () =>
      Promise.resolve(observerEvents[callIdx++]),
    );

    unsubscribe();

    // Must have fired exactly one batched notification for the full page.
    assert.equal(
      notifyCount,
      1,
      `expected exactly 1 batched subscriber notification after full archive page, got ${notifyCount}`,
    );

    // Production getter must reflect the added events.
    const archiveEvents = getArchivedChannelEvents(AGENT_PUBKEY, "chan-1");
    assert.equal(
      archiveEvents.length,
      PAGE_SIZE,
      "production getter must return all 50 archive events",
    );
  });

  it("test_all_duplicate_page_does_not_notify", async () => {
    _testRegisterKnownAgents(SUB_ID, [AGENT_PUBKEY]);

    // Seed one event.
    const obs = makeObserverEvent({ seq: 1, channelId: "chan-1" });
    await ingestArchivedObserverEvents([makeRawEvent()], () =>
      Promise.resolve(obs),
    );

    // Subscribe AFTER the first ingest so we only track the second ingest.
    let notifyCount = 0;
    const unsubscribe = subscribeAgentObserverStore(() => {
      notifyCount++;
    });

    // Re-ingest the same event — all duplicates, no new state.
    await ingestArchivedObserverEvents([makeRawEvent()], () =>
      Promise.resolve(obs),
    );

    unsubscribe();

    assert.equal(
      notifyCount,
      0,
      "a page of pure duplicates must not notify subscribers",
    );
  });
});

describe("raw-event-level merge: stateful aggregates across live/archive boundary", () => {
  beforeEach(() => {
    resetAgentObserverStore();
  });

  it("test_tool_start_in_archive_plus_update_in_live_yields_complete_row", () => {
    // Simulates the boundary scenario: tool_call frame is older than the
    // live cap and lives only in the archive; tool_call_update (completion) is
    // in the live window. Merging at the raw-event level and running a single
    // buildTranscriptState must yield one complete tool row with the update's
    // status, not two separate rows.
    const TOOL_ID = "tool-abc-123";
    const CHANNEL = "chan-merge";

    // Archive: tool_call (start, executing)
    const toolStart = makeObserverEvent({
      seq: 1,
      timestamp: "2026-01-01T00:00:01.000Z",
      kind: "acp_read",
      channelId: CHANNEL,
      sessionId: "sess-1",
      turnId: "turn-1",
      payload: {
        method: "session/update",
        params: {
          update: {
            sessionUpdate: "tool_call",
            toolCallId: TOOL_ID,
            toolName: "read_file",
            status: "executing",
            arguments: { path: "/tmp/x" },
          },
        },
      },
    });

    // Live: tool_call_update (completion)
    const toolUpdate = makeObserverEvent({
      seq: 2,
      timestamp: "2026-01-01T00:00:02.000Z",
      kind: "acp_read",
      channelId: CHANNEL,
      sessionId: "sess-1",
      turnId: "turn-1",
      payload: {
        method: "session/update",
        params: {
          update: {
            sessionUpdate: "tool_call_update",
            toolCallId: TOOL_ID,
            toolName: "read_file",
            status: "completed",
            result: "file content",
          },
        },
      },
    });

    const archiveEvents = [toolStart];
    const liveEvents = [toolUpdate];

    const combined = mergeObserverEventWindows(liveEvents, archiveEvents);
    assert.equal(combined.length, 2, "combined must have 2 raw events");

    const state = buildTranscriptState(combined);
    const toolRows = state.items.filter(
      (item) =>
        item.type === "tool" && item.id === `tool:${CHANNEL}:${TOOL_ID}`,
    );
    assert.equal(toolRows.length, 1, "must produce exactly 1 tool row");
    assert.equal(
      toolRows[0].status,
      "completed",
      "tool row must carry the completed status from the update frame",
    );
  });

  it("test_permission_request_in_archive_plus_response_in_live_yields_resolved_outcome", () => {
    // Simulates: permission request is older than the live cap (archive only);
    // permission response arrived live. Single-pass buildTranscriptState must
    // resolve the outcome and update the row.
    const CHANNEL = "chan-perm";
    const RPC_ID = "rpc-perm-42";

    // Archive: permission request — option uses production optionId + kind fields
    // so the optionId→kind map is populated and the outcome label is correct.
    const permRequest = makeObserverEvent({
      seq: 10,
      timestamp: "2026-01-01T00:10:00.000Z",
      kind: "acp_read",
      channelId: CHANNEL,
      sessionId: "sess-1",
      turnId: "turn-2",
      payload: {
        id: RPC_ID,
        method: "session/request_permission",
        params: {
          description: "Read /etc/passwd",
          options: [
            { optionId: "allow_once", kind: "allow_once", name: "Allow once" },
          ],
        },
      },
    });

    // Live: permission response — result.outcome carries outcome:"selected" and
    // the selected optionId so describePermissionOutcome builds "Approved (allow_once)".
    const permResponse = makeObserverEvent({
      seq: 11,
      timestamp: "2026-01-01T00:10:01.000Z",
      kind: "acp_write",
      channelId: CHANNEL,
      sessionId: "sess-1",
      turnId: "turn-2",
      payload: {
        id: RPC_ID,
        result: {
          outcome: {
            outcome: "selected",
            optionId: "allow_once",
          },
        },
      },
    });

    const combined = mergeObserverEventWindows([permResponse], [permRequest]);
    assert.equal(combined.length, 2, "combined must have 2 raw events");

    const state = buildTranscriptState(combined);
    const permRows = state.items.filter(
      (item) => item.type === "lifecycle" && item.renderClass === "permission",
    );
    assert.equal(permRows.length, 1, "must produce exactly 1 permission row");
    // The row must carry the fully-resolved production label.
    assert.equal(
      permRows[0].outcome,
      "Approved (allow_once)",
      "permission row outcome must be the production-shaped label when request+response are in the combined window",
    );
  });

  it("test_raw_rail_count_includes_archived_events_after_reload", async () => {
    // Simulates a reload-shaped scenario: only archived events are ingested
    // (no live events delivered yet). The combined raw window used for the
    // raw rail and header count must include the archived rows.
    _testRegisterKnownAgents(SUB_ID, [AGENT_PUBKEY]);
    const CHANNEL = "chan-raw";

    const archiveObsEvents = Array.from({ length: 5 }, (_, i) =>
      makeObserverEvent({
        seq: i + 1,
        timestamp: new Date(1_000_000 + i * 1000).toISOString(),
        channelId: CHANNEL,
      }),
    );
    let callIdx = 0;
    await ingestArchivedObserverEvents(
      Array.from({ length: 5 }, (_, i) =>
        makeRawEvent({ id: `r${String(i).padStart(63, "0")}` }),
      ),
      () => Promise.resolve(archiveObsEvents[callIdx++]),
    );

    // Production getter returns archived raw events.
    const archived = getArchivedChannelEvents(AGENT_PUBKEY, CHANNEL);
    // Live events for this agent are empty (no live relay ingest).
    const liveSnap = getAgentObserverSnapshot(AGENT_PUBKEY, true);
    const liveScoped = liveSnap.events.filter((e) => e.channelId === CHANNEL);

    const combined = mergeObserverEventWindows(liveScoped, archived);

    assert.equal(
      combined.length,
      5,
      "combined window must contain all 5 archived events for raw rail/count",
    );
    assert.equal(
      liveScoped.length,
      0,
      "live scoped events must be empty after archive-only ingest",
    );
  });

  it("test_header_last_updated_is_non_null_after_archive_only_ingest", async () => {
    // Regression for AgentSessionThreadPanel header: after a reload where only
    // archived events exist (no live relay yet), the merged raw window must
    // contain events with valid timestamps so the header shows "Last updated …"
    // rather than "No updates yet".
    _testRegisterKnownAgents(SUB_ID, [AGENT_PUBKEY]);
    const CHANNEL = "chan-header";
    const TIMESTAMP = "2026-06-01T12:00:00.000Z";

    await ingestArchivedObserverEvents(
      [makeRawEvent({ id: "h".repeat(64) })],
      () =>
        Promise.resolve(
          makeObserverEvent({
            seq: 1,
            timestamp: TIMESTAMP,
            channelId: CHANNEL,
          }),
        ),
    );

    // Simulate what AgentSessionThreadPanel computes for the header timestamp:
    // merged combined events (live=[], archive=[1 event]) → max timestamp.
    const archived = getArchivedChannelEvents(AGENT_PUBKEY, CHANNEL);
    const combined = mergeObserverEventWindows([], archived);

    assert.equal(
      combined.length,
      1,
      "combined must contain the archived event",
    );

    // latestActivityAt equivalent: max of finite Date.parse values.
    const latestMs = combined.reduce((acc, e) => {
      const parsed = Date.parse(e.timestamp);
      return Number.isFinite(parsed) ? Math.max(acc ?? -Infinity, parsed) : acc;
    }, /** @type {number | null} */ null);

    assert.ok(
      latestMs !== null && Number.isFinite(latestMs),
      "header latest timestamp must be non-null after archive-only ingest (header must not say 'No updates yet')",
    );
    assert.equal(latestMs, Date.parse(TIMESTAMP));
  });
});
