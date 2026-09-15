/**
 * Tests for getObservedAgentPubkeys — the whole-store "which agents have
 * observer data" snapshot.
 *
 * The function unions agents with live frames and agents with channel-scoped
 * archive windows, normalizes + sorts pubkeys, and caches the resulting array
 * per store notify-cycle so useSyncExternalStore callers get a stable
 * reference between store changes.
 *
 * Harness mirrors the existing observer-store tests
 * (ingestArchivedObserverEvents.test.mjs / observerTranscriptRetention.test.mjs):
 * resetAgentObserverStore between tests, live events seeded via
 * injectObserverEventsForE2E, archive events seeded via
 * ingestArchivedObserverEvents with _testRegisterKnownAgents and an injected
 * mock decrypt fn.
 */

import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import {
  getObservedAgentPubkeys,
  ingestArchivedObserverEvents,
  injectObserverEventsForE2E,
  resetAgentObserverStore,
  _testRegisterKnownAgents,
} from "@/features/agents/observerRelayStore.ts";

// ── Constants ─────────────────────────────────────────────────────────────────

// Chosen so sorted order is deterministic: "aa…" < "bb…" < "cc…".
const AGENT_A = "a".repeat(64);
const AGENT_B = "b".repeat(64);
const AGENT_C = "c".repeat(64);
const SUB_ID = "test-sub-observed";

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeObserverEvent(seq, overrides = {}) {
  return {
    seq,
    timestamp: new Date(1_760_000_000_000 + seq * 1000).toISOString(),
    kind: "acp_read",
    agentIndex: 0,
    channelId: null,
    sessionId: null,
    turnId: null,
    payload: null,
    ...overrides,
  };
}

/** Raw relay event that passes the archive-ingest guards for `agentPubkey`. */
function makeRawArchiveEvent(agentPubkey) {
  return {
    id: "e".repeat(64),
    pubkey: agentPubkey,
    created_at: 1000,
    kind: 24200,
    tags: [
      ["p", AGENT_A],
      ["agent", agentPubkey],
      ["frame", "telemetry"],
    ],
    content: "encrypted",
    sig: "s".repeat(128),
  };
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe("getObservedAgentPubkeys", () => {
  beforeEach(() => {
    resetAgentObserverStore();
  });

  it("test_empty_store_returns_empty_array", () => {
    assert.deepEqual(getObservedAgentPubkeys(), []);
  });

  it("test_live_event_for_one_agent_returns_normalized_pubkey", () => {
    // Seed with an uppercase pubkey — the store normalizes to lowercase.
    injectObserverEventsForE2E(AGENT_A.toUpperCase(), [makeObserverEvent(1)]);
    assert.deepEqual(getObservedAgentPubkeys(), [AGENT_A]);
  });

  it("test_live_events_for_two_agents_returns_sorted_pubkeys", () => {
    // Inject B before A to prove the result is sorted, not insertion-ordered.
    injectObserverEventsForE2E(AGENT_B, [makeObserverEvent(1)]);
    injectObserverEventsForE2E(AGENT_A, [makeObserverEvent(1)]);
    assert.deepEqual(getObservedAgentPubkeys(), [AGENT_A, AGENT_B]);
  });

  it("test_archive_only_agent_is_observed", async () => {
    // Agent C has NO live frames — only a channel-scoped archive window,
    // seeded through the real ingest path with a mock decrypt fn (the
    // test-only _decryptFn parameter, same seam ingestArchivedObserverEvents
    // tests use — no real decryption involved).
    _testRegisterKnownAgents(SUB_ID, [AGENT_C]);
    const archivedEvent = makeObserverEvent(1, { channelId: "chan-1" });
    await ingestArchivedObserverEvents([makeRawArchiveEvent(AGENT_C)], () =>
      Promise.resolve(archivedEvent),
    );
    assert.deepEqual(getObservedAgentPubkeys(), [AGENT_C]);
  });

  it("test_consecutive_calls_without_store_change_return_same_reference", () => {
    injectObserverEventsForE2E(AGENT_A, [makeObserverEvent(1)]);
    const ref1 = getObservedAgentPubkeys();
    const ref2 = getObservedAgentPubkeys();
    assert.equal(ref1, ref2, "cache must return the same array reference");

    // A store change invalidates the cache — new reference, updated contents.
    injectObserverEventsForE2E(AGENT_B, [makeObserverEvent(1)]);
    const ref3 = getObservedAgentPubkeys();
    assert.notEqual(ref3, ref1, "store change must produce a fresh snapshot");
    assert.deepEqual(ref3, [AGENT_A, AGENT_B]);
  });
});
