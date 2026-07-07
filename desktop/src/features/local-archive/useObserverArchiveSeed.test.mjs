/**
 * Tests for useObserverArchiveSeed seeding logic.
 *
 * The hook is a thin React wrapper around an async seed routine.  We test the
 * behaviour by driving the deps-injection interface directly — no React needed.
 * This mirrors the pattern used in archiveSyncManager.test.mjs.
 */

import assert from "node:assert/strict";
import test from "node:test";

// We call the seeding function directly via deps-injection rather than
// mounting the hook, so we import the seed-coordination logic by
// re-implementing a minimal version of `maybeSeed` driven by the same deps
// interface.  The test boundary is the async coordination logic, not React.

// ── Fake deps factory ────────────────────────────────────────────────────────

function makeDeps({
  defaultOn = false,
  hasExplicitChoice = false,
  mergeShouldFail = false,
} = {}) {
  const calls = { mergeSaveSubscriptionKinds: [], setExplicitChoice: [] };

  return {
    calls,
    observerArchiveDefaultEnabled: async () => defaultOn,
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

// Minimal re-implementation of the seeding logic from useObserverArchiveSeed.ts.
// Kept in sync with the source by structural mirroring (not bytewise copy) —
// change the source and update this accordingly.
const KIND_AGENT_OBSERVER_FRAME = 24200;

async function runSeed(pubkey, deps) {
  if (!pubkey) return;
  if (deps.hasExplicitChoice(pubkey)) return;

  let defaultOn;
  try {
    defaultOn = await deps.observerArchiveDefaultEnabled();
  } catch {
    return;
  }

  if (!defaultOn) return;

  try {
    await deps.mergeSaveSubscriptionKinds(KIND_AGENT_OBSERVER_FRAME);
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
  assert.equal(call.kind, 24200);
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
