import assert from "node:assert/strict";
import test from "node:test";

import {
  registerEffortNonce,
  resetAgentObserverStore,
  _testNonceGate,
  _testClearEffortNonceStorage,
} from "./observerRelayStore.ts";

const PUBKEY = "aabb";

// Minimal localStorage stub for the nonce-persistence tests.
function makeLocalStorage() {
  const store = new Map();
  return {
    get length() {
      return store.size;
    },
    key: (i) => [...store.keys()][i] ?? null,
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, value) => store.set(key, String(value)),
    removeItem: (key) => {
      store.delete(key);
    },
  };
}

// Reset store state AND localStorage before each test so nonce map is clean.
function setup() {
  // Install a fresh localStorage stub so each test has an isolated store.
  globalThis.localStorage = makeLocalStorage();
  resetAgentObserverStore();
}

// Full teardown: clear both the in-memory map and localStorage entries.
function fullReset() {
  _testClearEffortNonceStorage();
  resetAgentObserverStore();
}

// ── startup path (no nonce ever registered) ───────────────────────────────────

test("nonce gate: passes ack with no nonce when no nonce has been registered (startup path)", () => {
  setup();
  // Pre-any-pick: startup-applied effort acks carry no nonce.
  assert.equal(_testNonceGate(PUBKEY, undefined), true);
});

test("nonce gate: rejects ack WITH a nonce when no nonce has been registered", () => {
  setup();
  // Startup path: a nonce-bearing ack should not pass through the no-nonce gate.
  // This prevents a stale ack from an old session from persisting during startup.
  assert.equal(_testNonceGate(PUBKEY, "some-nonce"), false);
});

// ── post-registration path (nonce registered via registerEffortNonce) ─────────

test("nonce gate: passes ack with matching nonce once a nonce is registered", () => {
  setup();
  registerEffortNonce(PUBKEY, "nonce-42");
  assert.equal(_testNonceGate(PUBKEY, "nonce-42"), true);
});

test("nonce gate: rejects ack with mismatched nonce once a nonce is registered", () => {
  setup();
  registerEffortNonce(PUBKEY, "nonce-current");
  // Stale ack from a prior pick.
  assert.equal(_testNonceGate(PUBKEY, "nonce-old"), false);
});

test("nonce gate: rejects nonce-less ack once a nonce is registered (P3 bypass fix)", () => {
  setup();
  // P3: once a nonce has been registered, a nonce-less ok/cleared ack must NOT
  // pass through — it is a stale result from before the nonce system was in place
  // or from a superseded pick. Without this fix the ack would bypass the gate.
  registerEffortNonce(PUBKEY, "nonce-registered");
  assert.equal(_testNonceGate(PUBKEY, undefined), false);
});

// ── per-agent isolation ───────────────────────────────────────────────────────

test("nonce gate: different agents are isolated — no cross-registration", () => {
  setup();
  registerEffortNonce("agent-a", "nonce-a");
  // agent-b has no registered nonce → startup path applies.
  assert.equal(_testNonceGate("agent-b", undefined), true);
  assert.equal(_testNonceGate("agent-b", "nonce-a"), false);
});

// ── reset restores from localStorage (F3 durability) ─────────────────────────

test("resetAgentObserverStore restores nonce from localStorage — replay accepted post-reset", () => {
  setup();
  registerEffortNonce(PUBKEY, "nonce-durable");
  // Before reset: matching ack passes.
  assert.equal(_testNonceGate(PUBKEY, "nonce-durable"), true);

  // Reset clears the in-memory map, then immediately restores from localStorage.
  // An ack arriving after reset (e.g. a remote agent ack replayed after
  // community switch) should still be accepted.
  resetAgentObserverStore();
  assert.equal(
    _testNonceGate(PUBKEY, "nonce-durable"),
    true,
    "post-reset: matching nonce must pass (restored from localStorage)",
  );
  // A nonce-less ack must still be rejected (nonce is registered in storage).
  assert.equal(
    _testNonceGate(PUBKEY, undefined),
    false,
    "post-reset: nonce-less ack must be rejected when nonce is in storage",
  );
});

test("Desktop restart simulation — nonce persists, matching ack accepted", () => {
  setup();
  registerEffortNonce(PUBKEY, "nonce-restart");

  // Simulate Desktop restart: clear the in-memory map without touching
  // localStorage (the process dies and a new one starts, which calls
  // loadNoncesFromStorage on module init). Here we simulate that init by
  // calling _testClearEffortNonceStorage first to prove it's the storage that
  // provides the restoration, then calling fullReset + re-init via reset.
  //
  // Pattern: clear in-memory only, then reset (which calls loadNoncesFromStorage).
  resetAgentObserverStore();
  // localStorage still has the nonce — gate must accept the matching ack.
  assert.equal(
    _testNonceGate(PUBKEY, "nonce-restart"),
    true,
    "restart-simulation: matching nonce from localStorage must pass the gate",
  );
});

test("full reset (in-memory + storage) restores startup path", () => {
  setup();
  registerEffortNonce(PUBKEY, "nonce-registered");
  // Before clear: nonce-less ack is blocked.
  assert.equal(_testNonceGate(PUBKEY, undefined), false);

  // Clear both in-memory and storage — simulates a full logout/re-auth.
  fullReset();
  // After full reset: no nonce registered → startup path active again.
  assert.equal(
    _testNonceGate(PUBKEY, undefined),
    true,
    "full reset: startup path must be restored after clearing both memory and storage",
  );
});
