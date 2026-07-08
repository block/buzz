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
    const snap = getAgentObserverSnapshot(AGENT_PUBKEY, true);
    assert.equal(snap.events.length, 1);
    assert.equal(snap.events[0].seq, 1);
  });

  it("test_dedup_does_not_add_live_present_event", async () => {
    // Pre-seed a live event via E2E injection.
    const liveObs = makeObserverEvent({
      seq: 5,
      timestamp: "2026-01-01T00:00:05.000Z",
    });
    injectObserverEventsForE2E(AGENT_PUBKEY, [liveObs]);

    _testRegisterKnownAgents(SUB_ID, [AGENT_PUBKEY]);
    // Try to ingest an archived event with the SAME (seq, timestamp) — must be deduped.
    const archivedObs = makeObserverEvent({
      seq: 5,
      timestamp: "2026-01-01T00:00:05.000Z",
    });
    await ingestArchivedObserverEvents(
      [makeRawEvent()],
      makeDecrypt(archivedObs),
    );

    const snap = getAgentObserverSnapshot(AGENT_PUBKEY, true);
    assert.equal(
      snap.events.length,
      1,
      "dedup: duplicate seq+timestamp must not add a second entry",
    );
  });

  it("test_older_archived_event_sorts_before_live", async () => {
    // Pre-seed a newer live event.
    const liveObs = makeObserverEvent({
      seq: 2,
      timestamp: "2026-01-01T00:00:02.000Z",
    });
    injectObserverEventsForE2E(AGENT_PUBKEY, [liveObs]);

    _testRegisterKnownAgents(SUB_ID, [AGENT_PUBKEY]);
    // Ingest an older archived event.
    const archivedObs = makeObserverEvent({
      seq: 1,
      timestamp: "2026-01-01T00:00:01.000Z",
    });
    await ingestArchivedObserverEvents(
      [makeRawEvent()],
      makeDecrypt(archivedObs),
    );

    const snap = getAgentObserverSnapshot(AGENT_PUBKEY, true);
    assert.equal(snap.events.length, 2);
    // Ascending time order: older first.
    assert.equal(
      snap.events[0].seq,
      1,
      "older archived event must sort before newer live event",
    );
    assert.equal(snap.events[1].seq, 2);
  });

  it("test_multiple_events_ingested_in_order", async () => {
    _testRegisterKnownAgents(SUB_ID, [AGENT_PUBKEY]);
    // Three events: seq 3, 1, 2 — must end up sorted 1, 2, 3.
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
    const snap = getAgentObserverSnapshot(AGENT_PUBKEY, true);
    assert.equal(snap.events.length, 3);
    assert.deepEqual(
      snap.events.map((e) => e.seq),
      [1, 2, 3],
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
