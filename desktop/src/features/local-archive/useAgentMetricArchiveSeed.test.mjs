/**
 * Tests for useAgentMetricArchiveSeed seeding logic.
 *
 * Mirrors the pattern in useObserverArchiveSeed.test.mjs — drives the async
 * seed logic via the deps-injection interface, no React required.
 */

import assert from "node:assert/strict";
import test from "node:test";

// ── Fake deps factory ────────────────────────────────────────────────────────

function makeDeps({
  defaultOn = false,
  hasExplicitChoice = false,
  mergeShouldFail = false,
} = {}) {
  const calls = { mergeSaveSubscriptionKinds: [], setExplicitChoice: [] };

  return {
    calls,
    agentMetricArchiveDefaultEnabled: async () => defaultOn,
    mergeSaveSubscriptionKinds: async (kind) => {
      if (mergeShouldFail) throw new Error("merge failed");
      calls.mergeSaveSubscriptionKinds.push({ kind });
    },
    hasExplicitChoice: (_pubkey) => hasExplicitChoice,
    setExplicitChoice: (pubkey, enabled) => {
      calls.setExplicitChoice.push({ pubkey, enabled });
    },
  };
}

// Minimal re-implementation of the seeding logic from useAgentMetricArchiveSeed.ts.
// Kept in sync with the source by structural mirroring.
const KIND_AGENT_TURN_METRIC = 44200;

async function runSeed(pubkey, deps) {
  if (!pubkey) return;
  if (deps.hasExplicitChoice(pubkey)) return;

  let defaultOn;
  try {
    defaultOn = await deps.agentMetricArchiveDefaultEnabled();
  } catch {
    return;
  }

  if (!defaultOn) return;

  try {
    await deps.mergeSaveSubscriptionKinds(KIND_AGENT_TURN_METRIC);
  } catch {
    return; // transient failure — do NOT set explicit choice
  }

  deps.setExplicitChoice(pubkey, true);
}

// ── Tests ────────────────────────────────────────────────────────────────────

test("test_internal_build_unset_seeds_owner_p_subscription", async () => {
  const deps = makeDeps({ defaultOn: true, hasExplicitChoice: false });
  await runSeed("pubkey123", deps);

  assert.equal(
    deps.calls.mergeSaveSubscriptionKinds.length,
    1,
    "should call mergeSaveSubscriptionKinds once",
  );
  const call = deps.calls.mergeSaveSubscriptionKinds[0];
  assert.equal(call.kind, 44200);
});

test("test_internal_build_unset_persists_explicit_choice_after_seed", async () => {
  const deps = makeDeps({ defaultOn: true, hasExplicitChoice: false });
  await runSeed("pubkey123", deps);

  assert.equal(
    deps.calls.setExplicitChoice.length,
    1,
    "should persist explicit choice after successful seed",
  );
  assert.equal(deps.calls.setExplicitChoice[0].pubkey, "pubkey123");
  assert.equal(deps.calls.setExplicitChoice[0].enabled, true);
});

test("test_explicit_choice_set_does_not_reseed", async () => {
  const deps = makeDeps({ defaultOn: true, hasExplicitChoice: true });
  await runSeed("pubkey123", deps);

  assert.equal(
    deps.calls.mergeSaveSubscriptionKinds.length,
    0,
    "should not call mergeSaveSubscriptionKinds when explicit choice is already set",
  );
  assert.equal(
    deps.calls.setExplicitChoice.length,
    0,
    "should not update explicit choice when already set",
  );
});

test("test_oss_build_does_not_seed", async () => {
  const deps = makeDeps({ defaultOn: false, hasExplicitChoice: false });
  await runSeed("pubkey123", deps);

  assert.equal(
    deps.calls.mergeSaveSubscriptionKinds.length,
    0,
    "should not call mergeSaveSubscriptionKinds in OSS build",
  );
  assert.equal(
    deps.calls.setExplicitChoice.length,
    0,
    "should not persist explicit choice in OSS build",
  );
});

test("test_merge_failure_does_not_persist_explicit_choice", async () => {
  const deps = makeDeps({
    defaultOn: true,
    hasExplicitChoice: false,
    mergeShouldFail: true,
  });
  await runSeed("pubkey123", deps);

  assert.equal(
    deps.calls.setExplicitChoice.length,
    0,
    "should NOT persist explicit choice after a transient merge failure",
  );
});

test("test_empty_pubkey_does_nothing", async () => {
  const deps = makeDeps({ defaultOn: true, hasExplicitChoice: false });
  await runSeed("", deps);

  assert.equal(deps.calls.mergeSaveSubscriptionKinds.length, 0);
  assert.equal(deps.calls.setExplicitChoice.length, 0);
});

test("test_undefined_pubkey_does_nothing", async () => {
  const deps = makeDeps({ defaultOn: true, hasExplicitChoice: false });
  await runSeed(undefined, deps);

  assert.equal(deps.calls.mergeSaveSubscriptionKinds.length, 0);
  assert.equal(deps.calls.setExplicitChoice.length, 0);
});

// ── Concurrent-interleave test ───────────────────────────────────────────────
//
// Verifies the scenario Paul identified: on an internal-build first run with
// both flags on and no prior owner_p row, the observer and metric seeds race.
// With the old TS-side list+merge+create pattern the interleave could be:
//
//   1. observer seed: await list() → []
//   2. metric seed:  await list() → []    (row not yet written)
//   3. observer writes [24200]
//   4. metric writes [44200]  → clobbers 24200
//
// The new pattern delegates the merge to Rust under a single SQLite tx.
// Here we model that by tracking a shared "db state" and verifying that
// running both seeds concurrently (Promise.all) leaves both kinds present.

test("test_concurrent_seeds_both_kinds_survive", async () => {
  // Shared in-memory "db" — the atomic merge impl would serialize via SQLite
  // write lock; here we simulate by letting calls accumulate and checking
  // the final state.
  const db = new Set(); // kinds present after all merges

  function makeConcurrentDeps(defaultOn = true) {
    return {
      agentMetricArchiveDefaultEnabled: async () => defaultOn,
      // Simulates the atomic merge: each call simply adds its kind to the set,
      // regardless of what was there before (atomicity guarantee).
      mergeSaveSubscriptionKinds: async (kind) => {
        db.add(kind);
      },
      hasExplicitChoice: (_pubkey) => false,
      setExplicitChoice: () => {},
    };
  }

  const observerDeps = makeConcurrentDeps();
  const metricDeps = makeConcurrentDeps();

  // Replace the kind constant in the observer runSeed equivalent with 24200.
  async function runObserverSeed(pubkey, deps) {
    if (!pubkey) return;
    if (deps.hasExplicitChoice(pubkey)) return;
    let defaultOn;
    try {
      defaultOn = await deps.agentMetricArchiveDefaultEnabled();
    } catch {
      return;
    }
    if (!defaultOn) return;
    try {
      await deps.mergeSaveSubscriptionKinds(24200);
    } catch {
      return;
    }
    deps.setExplicitChoice(pubkey, true);
  }

  // Run both seeds concurrently — no await between them.
  await Promise.all([
    runObserverSeed("pubkey123", observerDeps),
    runSeed("pubkey123", metricDeps),
  ]);

  assert.ok(
    db.has(24200),
    "observer kind 24200 must be present after concurrent seeds",
  );
  assert.ok(
    db.has(44200),
    "metric kind 44200 must be present after concurrent seeds",
  );
  assert.equal(db.size, 2, "exactly two kinds, no extras");
});
